//! Resources (`docs/spec/slingshot/resources.md`, SS-RES): `ResourceSimulation`, the active
//! resources (processor sharing and FCFS from `simoxide-sched` on Slingshot's double clock),
//! linking resources and passive resources.

use super::*;
use simoxide_sched::{Clock, F64Secs, FcfsHead, ProcessorSharing, SchedulingPolicy};
use std::collections::VecDeque;

/// The run-time state of a resource of `cm.resources`.
pub(crate) enum ResRt {
    Ps(ProcessorSharing<u64, F64Secs>),
    Fcfs(FcfsHead<u64, F64Secs>),
}

/// A fresh resource of the active-resource table.
pub(super) fn new_res(r: &CRes) -> ResRt {
    match (r.kind, r.policy) {
        (ResKind::Processing(_), SchedulingPolicy::ProcessorSharing) => {
            ResRt::Ps(ProcessorSharing::new(r.instances.max(1)))
        }
        _ => ResRt::Fcfs(FcfsHead::new()),
    }
}

/// A `SimplePassiveResource`.
pub(crate) struct Passive {
    pub available: i64,
    /// Waiting jobs (FIFO) with their amounts.
    pub queue: VecDeque<(Rc<Rdr>, i64)>,
}

impl Engine<'_> {
    #[inline]
    fn now_t(&self) -> i64 {
        F64Secs::at_secs(self.now)
    }

    /// `ResourceSimulation`'s constructor (SS-RES-1, SS-RES-4.1): the active and linking
    /// resources, and the passive resources with their capacities (drawn before t = 0).
    pub(super) fn resources_init(&mut self) -> Result<(), Throw> {
        let cm = self.cm;
        let m = self.m;
        self.res = cm.resources.iter().map(new_res).collect();
        self.res_live = vec![true; cm.resources.len()];
        let Some(al) = m.allocations.iter().next() else {
            return Ok(());
        };
        self.passives_init(&al.contexts)?;
        // LinkingResourceTable: a link without a communication specification throws in
        // SimulatedLinkingResource's constructor (corpus x_sl_runconfigtest)
        for &l in &self.links {
            let lr = &m.linking_resources[l];
            if lr.spec_id.is_empty() && lr.throughput.is_missing() && lr.latency.is_missing() {
                return throw(
                    "NullPointerException",
                    format!(
                        "linking resource {} has no communication link resource specification",
                        lr.id
                    ),
                );
            }
        }
        Ok(())
    }

    /// `PassiveResourceTable.buildTable` / `buildPassiveResources` for the allocation contexts
    /// `contexts` (capacities drawn in order; an existing entry is replaced).
    pub(super) fn passives_init(&mut self, contexts: &[AllocationContextId]) -> Result<(), Throw> {
        let cm = self.cm;
        let m = self.m;
        for &ctx in contexts {
            let Some(ac) = m.allocation_contexts[ctx].assembly else {
                continue;
            };
            let Some(comp) = m.assembly_contexts[ac].component else {
                continue;
            };
            let ComponentKind::Basic {
                passive_resources, ..
            } = &m.components[comp].kind
            else {
                continue;
            };
            for &pr in passive_resources {
                let capacity = self.eval_i64(cm.pr_capacity[pr.index()], None, "capacity")?;
                let p = Passive {
                    available: capacity,
                    queue: VecDeque::new(),
                };
                match self.passive_idx.get(&(pr, ac)) {
                    Some(&i) => self.passives[i] = p,
                    None => {
                        self.passive_idx.insert((pr, ac), self.passives.len());
                        self.passives.push(p);
                    }
                }
            }
        }
        Ok(())
    }

    /// `publishState(job)`: (jobs at the resource, busy cores / cores).
    fn publish_state(&self, ri: ResIdx) -> (u64, f64) {
        match &self.res[ri as usize] {
            ResRt::Ps(s) => {
                let busy = (0..s.cores()).filter(|&k| s.queue_length(k) > 0).count();
                (s.len() as u64, busy as f64 / f64::from(s.cores()))
            }
            ResRt::Fcfs(s) => (s.len() as u64, if s.is_empty() { 0.0 } else { 1.0 }),
        }
    }

    /// The `ActiveResourceStateUpdated` of resource `ri`, if a monitor listens to it.
    fn state_event(&self, ri: ResIdx, out: &mut Out) {
        let i = ri as usize;
        if self.mon.state.get(i).is_some_and(|v| !v.is_empty())
            || self.mon.util.get(i).is_some_and(|v| !v.is_empty())
        {
            let (n, u) = self.publish_state(ri);
            out.push((Ev::ArState(ri, n, u), 0.0));
        }
    }

    fn active_res(&self, job: &Job) -> Option<ResIdx> {
        let JobKind::Active {
            rtype, container, ..
        } = &job.kind
        else {
            return None;
        };
        self.cm
            .container_res
            .get(&((*container)?, (*rtype)?))
            .copied()
            .filter(|&ri| self.res_live.get(ri as usize) == Some(&true))
    }

    /// `onResourceDemandRequested` (SS-RES-0.1, SS-RES-4.2).
    pub(super) fn on_demand_requested(&mut self, r: &Rc<Rdr>) -> HR {
        let cm = self.cm;
        let user = r.ctx.rpc.user.clone();
        if r.active {
            let CAct::Internal { demands, .. } = &cm.act[r.action.index()] else {
                unreachable!()
            };
            let (prog, rtype) = demands[r.demand];
            let top = self.top(&user)?;
            let d = self.eval_f64(prog, Some(&top), "demand")?;
            let container = self.alloc[r.ac.index()];
            let job = Rc::new(Job {
                id: self.job_id(),
                demand: Cell::new(d),
                kind: JobKind::Active {
                    rtype,
                    req: r.clone(),
                    container,
                },
            });
            return Ok(vec![if container.is_none() {
                (Ev::JobAbort(job), 0.0)
            } else {
                (Ev::JobInit(job), 0.0)
            }]);
        }
        let Some(pr) = r.passive else {
            return throw("NoSuchElementException", "No value present");
        };
        let Some(&pi) = self.passive_idx.get(&(pr, r.ac)) else {
            return Ok(Out::new());
        };
        let amount = self.passive_amount(r)?;
        let p = &mut self.passives[pi];
        if p.queue.is_empty() && amount <= p.available {
            p.available -= amount;
            let avail = p.available;
            return Ok(vec![
                (Ev::PAcq(r.clone()), 0.0),
                (Ev::PState(r.clone(), avail), 0.0),
            ]);
        }
        p.queue.push_back((r.clone(), amount));
        Ok(Out::new())
    }

    /// `createWaitingJob`: the amount of an acquire or release, `Long` in the user's top frame.
    fn passive_amount(&mut self, r: &Rdr) -> Result<i64, Throw> {
        let Some(prog) = self.cm.ss.passive_amount[r.action.index()] else {
            return throw("NullPointerException", "no parametric resource demand");
        };
        let top = self.top(&r.ctx.rpc.user)?;
        self.eval_i64(prog, Some(&top), "passive")
    }

    /// `onPassiveResourceReleased` (SS-RES-4.3).
    pub(super) fn on_passive_released(&mut self, r: &Rc<Rdr>) -> HR {
        let Some(pr) = r.passive else {
            return throw("NoSuchElementException", "No value present");
        };
        let Some(&pi) = self.passive_idx.get(&(pr, r.ac)) else {
            // "No such passive resource found!"
            return Ok(Out::new());
        };
        let amount = self.passive_amount(r)?;
        let p = &mut self.passives[pi];
        p.available += amount;
        let mut out = Out::new();
        while let Some((head, n)) = p.queue.front() {
            if *n > p.available {
                break;
            }
            p.available -= *n;
            out.push((Ev::PAcq(head.clone()), 0.0));
            p.queue.pop_front();
        }
        out.push((Ev::PState(r.clone(), p.available), 0.0));
        Ok(out)
    }

    /// `onJobInitiated` (SS-RES-0.2, SS-RES-5.2).
    pub(super) fn on_job_initiated(&mut self, job: &Rc<Job>) -> HR {
        let cm = self.cm;
        let now = self.now_t();
        match &job.kind {
            JobKind::Active { .. } => {
                let Some(ri) = self.active_res(job) else {
                    return Ok(vec![(Ev::JobAbort(job.clone()), 0.0)]);
                };
                let rate = self.eval_f64(cm.resources[ri as usize].rate, None, "rate")?;
                let calc = job.demand.get() / rate;
                job.demand.set(calc);
                let next = match &mut self.res[ri as usize] {
                    ResRt::Ps(s) => Some(s.process(now, job.id, calc, &mut ())),
                    ResRt::Fcfs(s) => {
                        let w = s.process(now, job.id, calc, &mut ());
                        if w.is_some() {
                            delay(calc)?;
                        }
                        w
                    }
                };
                self.live.insert(job.id, job.clone());
                let mut out = Out::new();
                self.state_event(ri, &mut out);
                if self
                    .mon
                    .demand
                    .get(ri as usize)
                    .is_some_and(|v| !v.is_empty())
                {
                    out.push((Ev::ArDemand(ri, calc), 0.0));
                }
                if let Some(w) = next {
                    // the completion of the shortest job (PS), of the head (FCFS)
                    out.push((Ev::JobProg(self.live[&w.job].clone(), w), 0.0));
                }
                Ok(out)
            }
            JobKind::Link { link, .. } => {
                let ri = cm.link_res[link];
                let r = &cm.resources[ri as usize];
                let tp = self.eval_f64(r.rate, None, "rate")?;
                let calc = job.demand.get() / tp;
                job.demand.set(calc);
                let latency = match r.latency {
                    Some(l) => self.eval_f64(l, None, "latency")?,
                    None => 0.0,
                };
                let d = calc + latency;
                job.demand.set(d);
                let ResRt::Fcfs(s) = &mut self.res[ri as usize] else {
                    unreachable!()
                };
                let next = s.process(now, job.id, d, &mut ());
                if next.is_some() {
                    delay(d)?;
                }
                self.live.insert(job.id, job.clone());
                Ok(next
                    .map(|w| (Ev::JobProg(self.live[&w.job].clone(), w), 0.0))
                    .into_iter()
                    .collect())
            }
        }
    }

    /// `onJobProgressed` (SS-RES-0.3, SS-RES-2.4/2.6, SS-RES-3.3, SS-RES-5.3).
    pub(super) fn on_job_progressed(&mut self, job: &Rc<Job>, w: &Wakeup<u64>) -> HR {
        let cm = self.cm;
        let m = self.m;
        let now = self.now_t();
        match &job.kind {
            JobKind::Active { .. } => {
                let Some(ri) = self.active_res(job) else {
                    return Ok(vec![(Ev::JobAbort(job.clone()), 0.0)]);
                };
                let c = match &mut self.res[ri as usize] {
                    ResRt::Ps(s) => s.on_wakeup(now, w, &mut ()),
                    ResRt::Fcfs(s) => s.on_wakeup(now, w, &mut ()),
                };
                let mut out = Out::new();
                if let Some(c) = c {
                    self.live.remove(&c.job);
                    out.push((Ev::JobFin(job.clone()), 0.0));
                    if let Some(n) = c.next {
                        let nj = self.live[&n.job].clone();
                        if matches!(self.res[ri as usize], ResRt::Fcfs(_)) {
                            delay(nj.demand.get())?;
                        }
                        out.push((Ev::JobProg(nj, n), 0.0));
                    }
                }
                self.state_event(ri, &mut out);
                Ok(out)
            }
            JobKind::Link { link, .. } => {
                let ri = cm.link_res[link];
                let mut site = String::from("link:");
                site.push_str(&m.linking_resources[*link].id);
                let r = self.rng.math_random(&site);
                let failure = cm.ss.link_failure[ri as usize];
                let ResRt::Fcfs(s) = &mut self.res[ri as usize] else {
                    unreachable!()
                };
                let c = s.on_wakeup(now, w, &mut ());
                if r < failure {
                    // abortJob: removed, the next job is not served (Q28)
                    s.fail_head(true);
                    self.live.remove(&job.id);
                    return Ok(vec![(Ev::JobAbort(job.clone()), 0.0)]);
                }
                let mut out = Out::new();
                if let Some(c) = c {
                    self.live.remove(&c.job);
                    out.push((Ev::JobFin(job.clone()), 0.0));
                    if let Some(n) = c.next {
                        let nj = self.live[&n.job].clone();
                        delay(nj.demand.get())?;
                        out.push((Ev::JobProg(nj, n), 0.0));
                    }
                }
                Ok(out)
            }
        }
    }

    /// `onJobFinished` (SS-RES-0.4).
    pub(super) fn on_job_finished(&mut self, job: &Rc<Job>) -> Out {
        match &job.kind {
            JobKind::Active { req, .. } => vec![(Ev::ArFinished(req.clone()), 0.0)],
            JobKind::Link { cow, .. } => vec![(Ev::CowOk(cow.clone()), 0.0)],
        }
    }

    /// `onJobAborted` (SS-RES-0.4).
    pub(super) fn on_job_aborted(&mut self, job: &Rc<Job>) -> Out {
        match &job.kind {
            JobKind::Active { req, .. } => vec![(Ev::RdAbort(req.clone()), 0.0)],
            JobKind::Link { cow, .. } => vec![(Ev::CowAbort(cow.clone()), 0.0)],
        }
    }

    /// `onCallOverWireRequested` (SS-ACT-7.1).
    pub(super) fn on_cow_requested(&mut self, cow: &Rc<Cow>) -> HR {
        let cm = self.cm;
        let m = self.m;
        let (Some(from), Some(to)) = (self.alloc[cow.from.index()], self.alloc[cow.to.index()])
        else {
            return Ok(vec![(Ev::CowOk(cow.clone()), 0.0)]);
        };
        if m.containers[from].id == m.containers[to].id {
            return Ok(vec![(Ev::CowOk(cow.clone()), 0.0)]);
        }
        let has = |l: LinkingResourceId, c: ContainerId| {
            m.linking_resources[l]
                .connected
                .iter()
                .any(|&x| m.containers[x].id == m.containers[c].id)
        };
        let link = self
            .links
            .iter()
            .copied()
            .find(|&l| has(l, from) && has(l, to))
            .or_else(|| {
                self.links
                    .iter()
                    .copied()
                    .find(|&l| has(l, from) || has(l, to))
            });
        let Some(link) = link else {
            return throw(
                "IllegalArgumentException",
                "The resource containers of the assembly contexts are not directly connected by a linking resource",
            );
        };
        // the payload: Σ BYTESIZE entries of the frame (SS-ACT-7.1)
        let mut d = 0.0;
        if let Some(vars) = &cow.vars {
            for (k, b) in frame::contents(vars) {
                if cm.keys.names[k as usize].ends_with("BYTESIZE") {
                    d += self.binding_to_f64(&b)?;
                }
            }
        }
        let job = Rc::new(Job {
            id: self.job_id(),
            demand: Cell::new(d),
            kind: JobKind::Link {
                link,
                cow: cow.clone(),
            },
        });
        Ok(vec![(Ev::JobInit(job), 0.0)])
    }
}
