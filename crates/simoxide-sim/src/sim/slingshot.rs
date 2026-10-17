//! The Slingshot flavour's own steps (`docs/spec/slingshot/`, rules `SS-*`; `DESIGN-SPD.md` §5.2).
//!
//! Everything here runs only in `Simulation<C, Slingshot>`: the shared code calls it under
//! `if F::SLINGSHOT`, a constant, so the SimuLizar flavour never contains these paths. The
//! interpreter, frames, calls and processes are shared; what differs is collected here:
//!
//! * resources (SS-RES): Slingshot's schedulers (processor sharing as SimuCom's, FCFS from the
//!   head, [`simoxide_sched::FcfsHead`]), zero demands as jobs, no HDD rates, the measured
//!   demand also when 0, stale PS completions (Q26), aborted users for missing resources and
//!   allocations, delay resources (fixed Q25);
//! * monitoring (SS-MEAS): instantaneous state and utilisation samples at every resource event,
//!   per resource or per core (Q34), passive-resource monitors per assembly context or merged
//!   (Q34); no sliding windows, no initial or final tuples;
//! * the `Math.random()` stream of branches and link failures (SS-RNG-1.2) in the exact RNG
//!   mode;
//! * links (SS-ACT-7, SS-RES-5): BYTESIZE payloads always, failure checks (Q28), the route
//!   fallback (Q37);
//! * passive resources (SS-RES-4): instances at the start, amounts of the action's demand (Q24),
//!   capacities without a frame (Q27).

use super::*;
use crate::flavour::Quirk;
use crate::javafmt::{push_json_double, push_json_str};

/// Error messages of the quirks that throw (the fast mode stops where Slingshot's handler
/// throws; Slingshot then disposes the handler, Q18, which only the exact mode reproduces).
const Q30_ABORT: &str = "IllegalStateException: User Interpretation Context not found! (Slingshot Q30: a demand of a composite's inner assembly context has no allocation and its abort fails)";

impl<'m, C: Compat, F: Flavour> Simulation<'m, C, F> {
    /// Is quirk `q` reproduced (Slingshot flavour)?
    #[inline]
    pub(super) fn quirk(&self, q: Quirk) -> bool {
        F::SLINGSHOT && self.cm.flavour.has(q)
    }

    /// Checks of the run configuration that only the Slingshot flavour has.
    pub(super) fn ss_check(cm: &CompiledModel, cfg: &mut SimConfig) -> Result<(), SimError> {
        let fail = |message: String| SimError {
            message,
            at_ns: 0,
            kind: SimErrorKind::Model,
        };
        let q = cm.flavour.quirks;
        if q.has(Quirk::Q18) {
            return Err(fail(
                "Slingshot quirk Q18 (a throwing handler is disposed and the run goes on) needs the Slingshot-exact mode (DESIGN-SPD.md X2); the fast mode stops with the error"
                    .into(),
            ));
        }
        if q.has(Quirk::Q17)
            && cm
                .act
                .iter()
                .any(|a| matches!(a, CAct::Internal { demands, .. } if demands.len() > 1))
        {
            return Err(fail(
                "Slingshot quirk Q17 (concurrent demands of one InternalAction) needs the Slingshot-exact mode (DESIGN-SPD.md X2)"
                    .into(),
            ));
        }
        if q.has(Quirk::Q32) {
            // simTime is the only stop condition (SS-RUN-5)
            cfg.max_measurements = -1;
        }
        Ok(())
    }

    // --------------------------------------------------------------------------- random

    /// A uniform of Slingshot's `Math.random()` stream (`purpose:id` on the tape): in the exact
    /// RNG mode `java.util.Random(seed + 1000003)` (SS-RNG-1.2), else the run's own stream.
    pub(super) fn ss_math(&mut self, purpose: &str, id: &str) -> f64 {
        if C::MODE != Mode::Exact {
            return self.st.rng.next_uniform();
        }
        let u = self.st.math.next_double();
        let i = self.st.math.count - 1;
        if let Some(b) = self.st.rng.tape_buf() {
            b.push_str("{\"k\":\"m\",\"i\":");
            b.push_str(&i.to_string());
            b.push_str(",\"u\":");
            push_json_double(b, u);
            b.push_str(",\"o\":");
            let mut o = String::with_capacity(purpose.len() + id.len() + 1);
            o.push_str(purpose);
            o.push(':');
            o.push_str(id);
            push_json_str(b, &o);
            b.push_str("}\n");
        }
        u
    }

    /// The provided-role call `(ia, ir, sig)` of an entry-level call through the system role
    /// `outer`, measured by Q33's key `(ia, outer, sig)` (SS-MEAS-2.3).
    pub(super) fn ss_prov_entry(
        &mut self,
        ia: AcRef,
        ir: RoleId,
        outer: RoleId,
        sig: SignatureId,
    ) -> u32 {
        // (a system role is no role of the component: the key cannot collide)
        if let Some(&id) = self.st.prov_ids.get(&(ia, outer, sig)) {
            return id;
        }
        let cm = self.cm;
        let m = &cm.model;
        let mut r = self.resolve_prov(ia, ir, sig);
        r.aser = cm
            .asmop_series
            .get(&op_key(
                &m.assembly_contexts[AssemblyContextId(ia)].id,
                &m.roles[outer].id,
                &m.signatures[sig].id,
            ))
            .copied();
        let id = self.st.provs.len() as u32;
        self.st.provs.push(r);
        self.st.prov_ids.insert((ia, outer, sig), id);
        id
    }

    // --------------------------------------------------------------------------- monitoring

    /// The active-resource monitors of resource `ri` sample its state (SS-MEAS-3): at every
    /// arrival, completion and (Q26) stale completion.
    pub(super) fn ss_res_samples(&mut self, ri: ResIdx) {
        let cm = self.cm;
        let Some(mons) = cm.ss.res_mon.get(ri as usize) else {
            return;
        };
        if mons.is_empty() {
            return;
        }
        let def = &cm.resources[ri as usize];
        let rt = &self.st.res[ri as usize];
        let (n, cores) = match &rt.sched {
            Some(s) => (s.jobs(), s.cores()),
            None => (rt.delay_set.len() as u64, 1),
        };
        let t = F::Clock::secs(self.st.now);
        for &(s, probe) in mons {
            let v = match probe {
                SsProbe::State => n as f64,
                SsProbe::Util => {
                    let busy = match def.policy {
                        SchedulingPolicy::ProcessorSharing => n.min(u64::from(cores)),
                        _ => u64::from(n > 0),
                    };
                    busy as f64 / f64::from(cores)
                }
                SsProbe::CoreState(k) => rt.sched.as_ref().map_or(0, |s| s.queue_length(k)) as f64,
                SsProbe::CoreUtil(k) => {
                    let q = rt.sched.as_ref().map_or(0, |s| s.queue_length(k));
                    if q > 0 { 1.0 } else { 0.0 }
                }
            };
            emit(
                &mut self.st.meas,
                &mut self.trace,
                self.st.cfg.store_measurements,
                self.st.now,
                s,
                t,
                v,
            );
        }
    }

    // --------------------------------------------------------------------------- resources

    /// A demand of `demand` on resource `ri` (`ResourceDemandRequested` → `JobInitiated`,
    /// SS-RES-0): returns true if the process now waits.
    pub(super) fn ss_consume(&mut self, s: u32, ri: ResIdx, demand: f64) -> R<bool> {
        let cm = self.cm;
        let def = &cm.resources[ri as usize];
        let origin = Origin::Plain(&def.origin);
        // the rate is drawn for every job, HDD read and write rates are ignored (SS-RES-0.2)
        let concrete = match def.kind {
            ResKind::Processing(_) => demand / self.eval_f64(def.rate, None, origin)?,
            ResKind::Link(_) => {
                let tp = self.eval_f64(def.rate, None, origin)?;
                let latency = match def.latency {
                    Some(l) => self.eval_f64(l, None, origin)?,
                    None => 0.0,
                };
                demand / tp + latency
            }
        };
        let pid = self.st.procs[s as usize].pid;
        if let Some(t) = self.trace.as_mut() {
            t.demand(
                self.st.now,
                pid,
                &def.type_id,
                &def.rc,
                &def.spec_name,
                &def.sched_name,
                demand,
                concrete,
            );
        }
        // the resource demand measurement, also when it is 0 (SS-MEAS-3.4)
        for &sid in &def.demand_series {
            emit(
                &mut self.st.meas,
                &mut self.trace,
                self.st.cfg.store_measurements,
                self.st.now,
                sid,
                F::Clock::secs(self.st.now),
                concrete,
            );
        }
        let k = self.pkey(s);
        let rt = &mut self.st.res[ri as usize];
        match rt.sched.as_mut() {
            None => {
                // a delay resource (the fixed behaviour of Q25: SimuLizar's delay scheduler)
                let Some(at) = F::Clock::checked_after(self.st.now, concrete) else {
                    return self.err(format!(
                        "IllegalArgumentException: invalid delay {concrete}"
                    ));
                };
                rt.delay_set.push(k);
                self.st.meas.events.insert(at, Ev::DelayRes(ri, k));
            }
            Some(sched) => {
                // FCFS serves the whole demand from the head: a negative demand is a negative
                // event delay, which Slingshot rejects (AbstractSimulationEvent)
                if !(concrete >= 0.0 && concrete.is_finite())
                    && !matches!(
                        sched,
                        ActiveResource::Ps(_) | ActiveResource::PsVirtualTime(_)
                    )
                {
                    return self.err(format!(
                        "IllegalArgumentException: the delay {concrete} of a job is not a finite non-negative number"
                    ));
                }
                let before = sched.pending_wakeup().map(|w| w.generation());
                let w = match sched.process(self.st.now, k, concrete, &mut ()) {
                    Ok(w) => w,
                    Err(e) => return self.err(e.0),
                };
                if before != Some(w.generation()) {
                    if self.quirk(Quirk::Q26) {
                        self.st.meas.events.set_timer_keep_stale(ri, w.at);
                    } else {
                        self.st.meas.events.set_timer(ri, w.at);
                    }
                }
            }
        }
        self.ss_res_samples(ri);
        Ok(true)
    }

    /// The pending completion of resource `ri` is due (`JobProgressed`, SS-RES-2.4, SS-RES-3.3;
    /// a link's failure check, SS-RES-5.3).
    pub(super) fn ss_wake(&mut self, ri: ResIdx) -> R<()> {
        let cm = self.cm;
        let rt = &mut self.st.res[ri as usize];
        let sched = rt.sched.as_mut().expect("scheduler");
        let w = sched.pending_wakeup().expect("pending wake-up");
        let c = match sched.on_wakeup(self.st.now, &w, &mut ()) {
            Ok(Some(c)) => c,
            Ok(None) => return Ok(()),
            Err(e) => return self.err(e.0),
        };
        let mut next = c.next;
        if let ResKind::Link(lid) = cm.resources[ri as usize].kind {
            let p = cm.ss.link_failure[ri as usize];
            let q28 = self.quirk(Quirk::Q28);
            if p > 0.0 || q28 {
                let id = &cm.model.linking_resources[lid].id;
                let r = self.ss_math("link", id);
                if r < p {
                    let (slot, _) = unkey(c.job);
                    self.st.procs[slot as usize].ss_flags |= SS_LINK_FAILED;
                    let sched = self.st.res[ri as usize].sched.as_mut().expect("scheduler");
                    next = sched.fail_head(q28);
                }
            }
        }
        if let Some(n) = next {
            self.st.meas.events.set_timer(ri, n.at);
        }
        self.ss_res_samples(ri);
        self.handoff(c.job)
    }

    /// A user whose demand cannot be served (no allocation, no resource of the type: SS-ACT-9.2,
    /// SS-RES-0.2, Q25) is aborted (`UserAborted`, SS-WL-5.1): a closed-workload user is replaced
    /// by a new one after a think time, an open-workload user is dropped. The interrupted
    /// request records nothing; what it holds stays held (SS-WL-5.2).
    pub(super) fn ss_abort(&mut self, s: u32, d: &ProcData, path: u32) -> R<Flow> {
        if self.quirk(Quirk::Q30) && path != ROOT_PATH && self.path_node(path).parent != ROOT_PATH {
            return self.err(Q30_ABORT);
        }
        match self.st.procs[s as usize].kind {
            PKind::ClosedUser => {
                let Some(&Cont::ClosedUser { scen, .. }) = d.conts.first() else {
                    return self.err("aborted closed-workload user without a workload driver");
                };
                let k = self.spawn(PKind::ClosedUser, Cont::ClosedUser { scen, st: 3 })?;
                self.activate(k)?;
                Ok(Flow::Finished)
            }
            PKind::OpenUser => Ok(Flow::Finished),
            _ => self.err(
                "a demand of a forked behaviour cannot be served (no allocation or resource): the Slingshot flavour cannot abort its user",
            ),
        }
    }

    /// A demand of the running process at its current assembly context: resolves allocation
    /// and resource like Slingshot (an unserved demand aborts the user) and consumes.
    /// `Ok(None)`: done without waiting; `Ok(Some(flow))`: return `flow`.
    pub(super) fn ss_demand(
        &mut self,
        s: u32,
        d: &mut ProcData,
        rtype: Option<ResourceTypeId>,
        v: f64,
    ) -> R<Option<Flow>> {
        let path = cur_path(d);
        let Some(c) = self.path_node(path).container else {
            return self.ss_abort(s, d, path).map(Some);
        };
        let Some(ri) = rtype.and_then(|t| self.cm.resource_in(c, t)) else {
            return self.ss_abort(s, d, path).map(Some);
        };
        if self.ss_consume(s, ri, v)? {
            return Ok(Some(self.wait_consumed(d, ri)));
        }
        Ok(None)
    }

    /// A transfer between the containers of a call over the wire (SS-ACT-7.1): the payload's
    /// BYTESIZE entries, always (SS-ACT-7.2); none within one container or when a side has no
    /// container. Returns the link if the process now waits.
    pub(super) fn ss_transmit(
        &mut self,
        s: u32,
        src: ContainerId,
        dst: ContainerId,
        payload: Option<&Frame>,
    ) -> R<Option<ResIdx>> {
        let cm = self.cm;
        if src == dst || src == NO_CONTAINER || dst == NO_CONTAINER {
            return Ok(None);
        }
        let mut b = 0.0f64;
        if let Some(f) = payload
            && f.any_key(|k| cm.keys.bytesize[k as usize])
        {
            f.visit_contents::<C::Order, _>(&cm.keys, |k, v| {
                if cm.keys.bytesize[k as usize] {
                    b += self.binding_to_f64(v)?;
                }
                Ok::<(), Box<SimError>>(())
            })?;
        }
        let m = &cm.model;
        let n = m.containers.len();
        let direct = match cm.route_res.get(src.index() * n + dst.index()) {
            Some(&r) => (r != u32::MAX).then_some(r),
            None => cm
                .links
                .iter()
                .copied()
                .find(|&l| {
                    let c = &m.linking_resources[l].connected;
                    c.contains(&src) && c.contains(&dst)
                })
                .map(|l| cm.link_res[&l]),
        };
        let ri = match direct {
            Some(r) => r,
            None => {
                // Q37: the links touching either container (meant for removed containers); none
                // at all throws
                let fallback = self
                    .quirk(Quirk::Q37)
                    .then(|| {
                        cm.links.iter().copied().find(|&l| {
                            let c = &m.linking_resources[l].connected;
                            c.contains(&src) || c.contains(&dst)
                        })
                    })
                    .flatten();
                match fallback {
                    Some(l) => cm.link_res[&l],
                    None => {
                        return self.err(format!(
                            "IllegalArgumentException: no linking resource connects the containers {} and {}",
                            m.containers[src].id, m.containers[dst].id
                        ));
                    }
                }
            }
        };
        Ok(self.ss_consume(s, ri, b)?.then_some(ri))
    }

    /// The process's last transfer failed (SS-ACT-7.3): it is requested again.
    #[inline]
    pub(super) fn ss_take_link_failure(&mut self, s: u32) -> bool {
        let p = &mut self.st.procs[s as usize];
        let failed = p.ss_flags & SS_LINK_FAILED != 0;
        p.ss_flags &= !SS_LINK_FAILED;
        failed
    }

    // --------------------------------------------------------------------------- passive

    /// The passive resources of every allocated basic component instance, created before the
    /// simulation starts (`ResourceSimulation`'s constructor, SS-RES-4.1).
    pub(super) fn ss_create_passives(&mut self) -> R<()> {
        let cm = self.cm;
        for path in 1..cm.ac_paths.len() as u32 {
            let node = &cm.ac_paths[path as usize];
            if node.container.is_none() || node.ac == SYSTEM_AC {
                continue;
            }
            let Some(comp) = cm.model.assembly_contexts[AssemblyContextId(node.ac)].component
            else {
                continue;
            };
            if !matches!(cm.model.components[comp].kind, ComponentKind::Basic { .. })
                || cm.model.components[comp].passive_resources().is_empty()
            {
                continue;
            }
            self.ss_component_instance(path)?;
        }
        Ok(())
    }

    /// The passive resources of the basic component instance at `path` (SS-RES-4.1): capacities
    /// evaluated once, in the component instance's frame (fixed Q27) or without a frame (Q27).
    pub(super) fn ss_component_instance(&mut self, path: u32) -> R<()> {
        if self
            .st
            .comps
            .get(path as usize)
            .is_some_and(|c| c.is_some())
        {
            return Ok(());
        }
        let cm = self.cm;
        let m = &cm.model;
        let ac = self.path_node(path).ac;
        let acid = AssemblyContextId(ac);
        let Some(comp) = m.assembly_contexts[acid].component else {
            return self.err("assembly context without component");
        };
        let frame = if self.quirk(Quirk::Q27) {
            None
        } else {
            let mut fc = self.st.pool.frame(None);
            self.fill(
                &cm.comp_params[comp.index()],
                None,
                frame_mut(&mut fc),
                Origin::Plain("?"),
            )?;
            let mut fa = self.st.pool.frame(Some(fc.clone()));
            self.fill(
                &cm.ac_params[acid.index()],
                Some(&fc),
                frame_mut(&mut fa),
                Origin::Plain("?"),
            )?;
            Some(fa)
        };
        let mut passives = Vec::new();
        for &pr in m.components[comp].passive_resources() {
            let v = self.eval(
                cm.pr_capacity[pr.index()],
                frame.as_deref(),
                Origin::Plain("capacity"),
            )?;
            let capacity = match v.convert(simoxide_stoex::Expected::Long) {
                Ok(simoxide_stoex::Converted::Long(l)) => l,
                _ => return self.err(format!("capacity {v:?} is no Long")),
            };
            let ss_mon = cm
                .ss
                .passive_mon
                .iter()
                .filter(|p| p.pr == pr && p.ac.is_none_or(|a| a == acid))
                .map(|p| (p.series, p.metric))
                .collect();
            let idx = self.st.passives.len() as u32;
            self.st.passives.push(PassiveInst {
                pr,
                ac,
                res: PassiveResource::new(capacity.max(0) as u64),
                capacity,
                state: None,
                waiting: None,
                holding: None,
                wait_start: Vec::new(),
                hold_start: Vec::new(),
                ss_mon,
            });
            passives.push((pr, idx));
        }
        self.st.pool.recycle(frame);
        if self.st.comps.len() <= path as usize {
            self.st.comps.resize_with(path as usize + 1, || None);
        }
        self.st.comps[path as usize] = Some(CompInst { passives });
        Ok(())
    }

    /// The amount of an acquire or release (SS-ACT-8.1, SS-RES-4.2/4.3): the action's first
    /// `ParametricResourceDemand`, evaluated as `Long` in the user's top frame; one unit without
    /// one (fixed Q24).
    fn ss_passive_amount(&mut self, d: &ProcData, a: ActionId) -> R<u64> {
        let cm = self.cm;
        match cm.ss.passive_amount.get(a.index()).copied().flatten() {
            Some(p) => {
                let v = self.eval(p, d.frames.last().map(|f| &**f), Origin::Plain("passive"))?;
                match v.convert(simoxide_stoex::Expected::Long) {
                    Ok(simoxide_stoex::Converted::Long(l)) => Ok(l.max(0) as u64),
                    _ => self.err(format!("passive resource amount {v:?} is no Long")),
                }
            }
            None if self.quirk(Quirk::Q24) => self.err(format!(
                "NoSuchElementException: {} {} has no ParametricResourceDemand (Slingshot Q24)",
                cm.action_type[a.index()],
                cm.action_ids[a.index()]
            )),
            None => Ok(1),
        }
    }

    /// Emits the state of passive instance `pi` (`PassiveResourceStateUpdated`).
    fn ss_passive_state(&mut self, pi: usize) {
        let t = F::Clock::secs(self.st.now);
        let inst = &self.st.passives[pi];
        let avail = inst.res.available() as f64;
        for &(s, metric) in &inst.ss_mon {
            if metric == SsPassiveMetric::State {
                emit(
                    &mut self.st.meas,
                    &mut self.trace,
                    self.st.cfg.store_measurements,
                    self.st.now,
                    s,
                    t,
                    avail,
                );
            }
        }
    }

    /// Acquire (SS-ACT-8.2, SS-RES-4.2): returns true if the process now waits.
    pub(super) fn ss_acquire(
        &mut self,
        s: u32,
        d: &ProcData,
        a: ActionId,
        pr: Option<PassiveResourceId>,
    ) -> R<bool> {
        let pi = self.passive_of(d, pr)? as usize;
        let amount = self.ss_passive_amount(d, a)?;
        let k = self.pkey(s);
        let now = self.st.now;
        let inst = &mut self.st.passives[pi];
        // the waiting time starts at the request (before the resource handles it)
        if inst.ss_mon.iter().any(|m| m.1 == SsPassiveMetric::Waiting) {
            inst.wait_start.push((k, now));
        }
        let mut l = SsPassiveL::<F::Clock> {
            meas: &mut self.st.meas,
            trace: &mut self.trace,
            store: self.st.cfg.store_measurements,
            now,
            mons: &inst.ss_mon,
            wait_start: &mut inst.wait_start,
            hold_start: &mut inst.hold_start,
            woken: Vec::new(),
            clock: std::marker::PhantomData,
        };
        let granted = inst.res.acquire(k, amount, &mut l);
        if granted {
            self.ss_passive_state(pi);
        }
        Ok(!granted)
    }

    /// Release (SS-ACT-8.3, SS-RES-4.3): the amount is evaluated again; waiting requests are
    /// granted in FIFO order; the releasing process goes on.
    pub(super) fn ss_release(
        &mut self,
        s: u32,
        d: &ProcData,
        a: ActionId,
        pr: Option<PassiveResourceId>,
    ) -> R<()> {
        let pi = self.passive_of(d, pr)? as usize;
        let amount = self.ss_passive_amount(d, a)?;
        let k = self.pkey(s);
        let now = self.st.now;
        let inst = &mut self.st.passives[pi];
        let mut l = SsPassiveL::<F::Clock> {
            meas: &mut self.st.meas,
            trace: &mut self.trace,
            store: self.st.cfg.store_measurements,
            now,
            mons: &inst.ss_mon,
            wait_start: &mut inst.wait_start,
            hold_start: &mut inst.hold_start,
            woken: Vec::new(),
            clock: std::marker::PhantomData,
        };
        inst.res.release(k, amount, &mut l);
        let woken = std::mem::take(&mut l.woken);
        self.ss_passive_state(pi);
        for w in woken {
            self.activate(w)?;
        }
        Ok(())
    }

    // --------------------------------------------------------------------------- stop

    /// The end of a Slingshot run (SS-RUN-3/4): `SimulationFinished` clears the queues; no
    /// drain, no final samples.
    pub(super) fn ss_finalise(&mut self) {
        self.st.running = false;
        if self.st.ss_time_stop
            || (self.st.cfg.max_sim_time > 0 && self.st.meas.events.next_time().is_none())
        {
            self.st.now = F::Clock::at_secs(self.st.cfg.max_sim_time as f64);
        }
        let now = self.st.now;
        if let Some(t) = self.trace.as_mut() {
            t.stop(now);
        }
        for ri in 0..self.st.res.len() {
            if let Some(s) = self.st.res[ri].sched.as_mut() {
                s.stop();
            }
            self.st.meas.events.clear_timer(ri as ResIdx);
        }
        let (u, n) = (self.st.rng.count(), self.st.meas.m.count);
        if let Some(t) = self.trace.as_mut() {
            t.finish_line(now, u, n);
        }
    }
}

/// The `left` of a SEFF loop that runs its body once and then throws (Q19 with count 0).
pub(super) const Q19_ONCE: i32 = i32::MIN;
/// Q19, a SEFF loop with count 0 after its body (SS-ACT-6.2).
pub(super) const Q19_SEFF: &str = "NoSuchElementException: the body of a LoopAction with iteration count 0 has finished and the loop asks for another iteration (Slingshot Q19)";
/// Q19, a negative SEFF loop count.
pub(super) const Q19_NEGATIVE: &str =
    "IllegalArgumentException: negative iteration count of a LoopAction (Slingshot Q19)";
/// Q19, a usage-model loop count of 0 or less (SS-WL-4.3).
pub(super) const Q19_USAGE: &str = "IllegalArgumentException: the iteration count of a usage-model Loop must be positive (Slingshot Q19)";
/// Q22 at the end of a request (SS-ACT-6.3, SS-WL-4.4).
pub(super) const Q22_STACK: &str = "IllegalArgumentException: UserRequestFinished: the user's stack has an extra frame left by a CollectionIteratorAction (Slingshot Q22)";

/// Container of an unallocated side of a call (SS-ACT-7.1: no transfer).
pub(super) const NO_CONTAINER: ContainerId = ContainerId(u32::MAX);

/// Listener of Slingshot's passive-resource monitors: waiting and holding times
/// (SS-MEAS-4.2) and wake-ups.
struct SsPassiveL<'a, K: Clock> {
    meas: &'a mut Sink,
    trace: &'a mut Option<TraceOut>,
    store: bool,
    now: i64,
    mons: &'a [(SeriesId, SsPassiveMetric)],
    wait_start: &'a mut Vec<(u64, i64)>,
    hold_start: &'a mut Vec<(u64, i64)>,
    woken: Vec<u64>,
    clock: std::marker::PhantomData<K>,
}

impl<K: Clock> SsPassiveL<'_, K> {
    fn emit_metric(&mut self, metric: SsPassiveMetric, t0: i64) {
        let t = K::secs(self.now);
        let v = t - K::secs(t0);
        for &(s, m) in self.mons {
            if m == metric {
                emit(self.meas, self.trace, self.store, self.now, s, t, v);
            }
        }
    }
}

impl<K: Clock> PassiveListener<u64> for SsPassiveL<'_, K> {
    fn acquired(&mut self, job: u64, _num: u64, _available: i64) {
        if let Some(pos) = self.wait_start.iter().position(|x| x.0 == job) {
            let (_, t0) = self.wait_start.remove(pos);
            self.emit_metric(SsPassiveMetric::Waiting, t0);
        }
        if self.mons.iter().any(|m| m.1 == SsPassiveMetric::Holding) {
            self.hold_start.push((job, self.now));
        }
    }

    fn released(&mut self, job: u64, _num: u64, _available: i64) {
        if let Some(pos) = self.hold_start.iter().position(|x| x.0 == job) {
            let (_, t0) = self.hold_start.remove(pos);
            self.emit_metric(SsPassiveMetric::Holding, t0);
        }
    }

    fn wake(&mut self, job: u64) {
        self.woken.push(job);
    }
}
