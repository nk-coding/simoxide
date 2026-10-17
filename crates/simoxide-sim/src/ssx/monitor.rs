//! Monitoring (`docs/spec/slingshot/monitoring.md`, SS-MEAS): the calculators the monitor
//! repository asks for, and the monitor behaviours that feed their probes.

use super::*;

/// The calculators of the monitor repository, in `MeasurementSpecificationVisited` order
/// (SS-MEAS-1.1): one series per (measuring point key, metric).
pub(super) fn build(cm: &CompiledModel, meas: &mut Measurements) -> Monitors {
    let m = &cm.model;
    let mut mon = Monitors {
        scenario: vec![None; cm.scenarios.len()],
        state: vec![Vec::new(); cm.resources.len()],
        util: vec![Vec::new(); cm.resources.len()],
        demand: vec![Vec::new(); cm.resources.len()],
        ..Monitors::default()
    };
    let Some(repo) = m.monitor_repositories.iter().next() else {
        return mon;
    };
    for &mid in &repo.monitors {
        mon.add(cm, meas, mid, None);
    }
    mon
}

impl Monitors {
    /// The calculators of monitor `mid` (`MonitorModelVisitor`, SS-MEAS-1.1), at the start or
    /// for a monitor an SPD adjustment cloned (`MonitorRepositoryInterpreterBehavior.
    /// onModelAdjusted`). `alias`: the measuring-point keys of monitors created during the run
    /// name created elements by their aliases (`Keys.alias`).
    pub(super) fn add(
        &mut self,
        cm: &CompiledModel,
        meas: &mut Measurements,
        mid: MonitorId,
        alias: Option<&HashMap<Box<str>, Box<str>>>,
    ) {
        let m = &cm.model;
        let n = cm.resources.len();
        for v in [&mut self.state, &mut self.util, &mut self.demand] {
            if v.len() < n {
                v.resize(n, Vec::new());
            }
        }
        if self.scenario.len() < cm.scenarios.len() {
            self.scenario.resize(cm.scenarios.len(), None);
        }
        let mut series = |meas: &mut Measurements, key: String, metric: &'static str| -> SeriesId {
            let shown = match alias {
                Some(a) if !a.is_empty() => {
                    let mut k = key.clone();
                    for (id, al) in a {
                        if k.contains(&**id) {
                            k = k.replace(&**id, al);
                        }
                    }
                    k
                }
                _ => key.clone(),
            };
            *self.keys.entry((key.clone(), metric)).or_insert_with(|| {
                meas.series.push(SeriesDef {
                    mp: shown.clone().into(),
                    metric,
                });
                meas.rows.push(Vec::new());
                let s = (meas.series.len() - 1) as SeriesId;
                if shown != key {
                    self.raw_keys.insert(s, key.into());
                }
                s
            })
        };
        let mo = &m.monitors[mid];
        if !mo.activated {
            return;
        }
        let Some(mp) = mo.measuring_point.map(|x| &m.measuring_points[x]) else {
            return;
        };
        for spec in &mo.specifications {
            let metric = metric_id(m, spec);
            match &mp.kind {
                MeasuringPointKind::ActiveResource {
                    resource: Some(r), ..
                } => {
                    let (name, slot) = match metric {
                        Some(metric_ids::STATE_OF_ACTIVE_RESOURCE) => {
                            (metric_names::STATE_OF_ACTIVE_RESOURCE, 0)
                        }
                        Some(metric_ids::RESOURCE_DEMAND) => (metric_names::RESOURCE_DEMAND, 2),
                        Some(metric_ids::UTILIZATION) => (metric_names::UTILIZATION, 1),
                        _ => continue,
                    };
                    let Some(key) = ss_mp_key(m, mp) else {
                        continue;
                    };
                    let s = series(meas, key, name);
                    // probes per ProcessingResourceSpecification (ActiveResourceProbeTable)
                    let idx = cm.resources.iter().position(|x| {
                        matches!(x.kind, ResKind::Processing(p)
                            if m.processing_resources[p].id == m.processing_resources[*r].id)
                    });
                    if let Some(i) = idx {
                        match slot {
                            0 => self.state[i].push(s),
                            1 => self.util[i].push(s),
                            _ => self.demand[i].push(s),
                        }
                    }
                }
                MeasuringPointKind::AssemblyPassiveResource {
                    passive_resource: Some(p),
                    ..
                } => {
                    let (name, kind) = match metric {
                        Some(metric_ids::WAITING_TIME) => (metric_names::WAITING_TIME, 0),
                        Some(metric_ids::HOLDING_TIME) => (metric_names::HOLDING_TIME, 1),
                        Some(metric_ids::STATE_OF_PASSIVE_RESOURCE) => {
                            (metric_names::STATE_OF_PASSIVE_RESOURCE, 2)
                        }
                        _ => continue,
                    };
                    let Some(key) = ss_mp_key(m, mp) else {
                        continue;
                    };
                    let s = series(meas, key, name);
                    // probes per passive resource id (PassiveResourceProbeTable, Q34)
                    let probes = self
                        .passive
                        .entry(m.passive_resources[*p].id.clone())
                        .or_default();
                    match kind {
                        0 | 1 => {
                            let ci = self.calcs.len();
                            self.calcs.push(StartStop {
                                series: s,
                                open: Vec::new(),
                            });
                            if kind == 0 {
                                probes.waiting.push(ci);
                            } else {
                                probes.holding.push(ci);
                            }
                            probes.acquired.push((ci, kind == 0));
                        }
                        _ => probes.state.push(s),
                    }
                }
                MeasuringPointKind::AssemblyOperation {
                    assembly: Some(ac),
                    role: Some(role),
                    signature: Some(sig),
                } if metric == Some(metric_ids::RESPONSE_TIME)
                    && m.roles[*role].kind == RoleKind::OperationProvided =>
                {
                    let Some(key) = ss_mp_key(m, mp) else {
                        continue;
                    };
                    let s = series(meas, key, metric_names::RESPONSE_TIME);
                    let ci = self.calcs.len();
                    self.calcs.push(StartStop {
                        series: s,
                        open: Vec::new(),
                    });
                    self.asmop.insert(
                        (
                            m.assembly_contexts[*ac].id.clone(),
                            m.roles[*role].id.clone(),
                            m.signatures[*sig].id.clone(),
                        ),
                        ci,
                    );
                }
                MeasuringPointKind::UsageScenario { scenario: Some(sc) }
                    if metric == Some(metric_ids::RESPONSE_TIME) =>
                {
                    let Some(key) = ss_mp_key(m, mp) else {
                        continue;
                    };
                    let s = series(meas, key, metric_names::RESPONSE_TIME);
                    let ci = self.calcs.len();
                    self.calcs.push(StartStop {
                        series: s,
                        open: Vec::new(),
                    });
                    // userProbesMap.put(scenario id): the last monitor of a scenario wins
                    for (i, x) in cm.scenarios.iter().enumerate() {
                        if m.usage_scenarios[x.id].id == m.usage_scenarios[*sc].id {
                            self.scenario[i] = Some(ci);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl Engine<'_> {
    /// The first probe of a start-stop calculator.
    fn calc_start(&mut self, ci: usize, ctx: RequestCtx) -> Result<(), Throw> {
        let c = &mut self.mon.calcs[ci];
        if c.open.iter().any(|x| x.0 == ctx) {
            return throw(
                "IllegalStateException",
                "First measurement to the same context arrived while previous series of the same context did not complete.",
            );
        }
        c.open.push((ctx, self.now));
        Ok(())
    }

    /// The last probe: measures `now - start` at `now` (or nothing without a start).
    fn calc_stop(&mut self, ci: usize, ctx: RequestCtx) {
        let c = &mut self.mon.calcs[ci];
        let Some(pos) = c.open.iter().position(|x| x.0 == ctx) else {
            return;
        };
        let (_, t0) = c.open.remove(pos);
        let s = c.series;
        self.measure(s, self.now, self.now - t0);
    }

    // --------------------------------------------------------------------------- usage

    /// `onUsageScenarioStarted<Start>` (SS-MEAS-2.1).
    pub(super) fn mon_usage_started(&mut self, c: &Rc<UCtx>, a: UserActionId) -> HR {
        if matches!(self.m.user_actions[a].kind, UserActionKind::Start)
            && c.beh.kind == UBehKind::Root
            && let Some(ci) = self.mon.scenario[c.scenario]
        {
            let ctx = RequestCtx {
                user: c.user.id,
                caller: None,
            };
            self.calc_start(ci, ctx)?;
        }
        Ok(Out::new())
    }

    /// `onUsageScenarioFinished<Stop>`.
    pub(super) fn mon_usage_finished(&mut self, c: &Rc<UCtx>, a: UserActionId) -> HR {
        if matches!(self.m.user_actions[a].kind, UserActionKind::Stop)
            && c.beh.kind == UBehKind::Root
            && let Some(ci) = self.mon.scenario[c.scenario]
        {
            let ctx = RequestCtx {
                user: c.user.id,
                caller: None,
            };
            self.calc_stop(ci, ctx);
        }
        Ok(Out::new())
    }

    // --------------------------------------------------------------------------- operations

    /// `AssemblyOperationCompoundKey.of(context)`: throws without a user request and a call
    /// over the wire (Q31, SS-MEAS-2.4).
    fn asmop_calc(&self, c: &SeffCtx) -> Result<Option<usize>, Throw> {
        let m = self.m;
        let sig = if let Some(r) = &c.rpc.request {
            r.sig
        } else if let Some(cow) = &c.cow {
            cow.entry.sig
        } else {
            return throw(
                "IllegalArgumentException",
                "Cannot determine Signature of SEFFInterpretationContext org.palladiosimulator.analyzer.slingshot.behavior.systemsimulation.entities.seff.SEFFInterpretationContext@*, as their is neither a UserRequest, nor a CallOverWireRequest.",
            );
        };
        let Some(role) = c.rpc.role else {
            return throw("NullPointerException", "provided role is null");
        };
        if m.roles[role].kind != RoleKind::OperationProvided {
            return Ok(None);
        }
        let Some(sig) = sig else {
            return throw("NullPointerException", "signature is null");
        };
        let key = (
            m.assembly_contexts[c.ac].id.clone(),
            m.roles[role].id.clone(),
            m.signatures[sig].id.clone(),
        );
        Ok(self.mon.asmop.get(&key).copied())
    }

    fn op_ctx(c: &SeffCtx) -> RequestCtx {
        RequestCtx {
            user: c.rpc.user.id,
            caller: c.caller.as_ref().map(|x| x.id),
        }
    }

    /// `onOperationCallStarted<StartAction>` (SS-MEAS-2.3).
    pub(super) fn mon_op_started(&mut self, c: &Rc<SeffCtx>, a: ActionId) -> HR {
        if !matches!(self.m.actions[a].kind, ActionKind::Start) || c.holder.borrow().child {
            return Ok(Out::new());
        }
        if let Some(ci) = self.asmop_calc(c)? {
            self.calc_start(ci, Self::op_ctx(c))?;
        }
        Ok(Out::new())
    }

    /// `onOperationCallStopped<StopAction>`.
    pub(super) fn mon_op_stopped(&mut self, c: &Rc<SeffCtx>, a: ActionId) -> HR {
        if !matches!(self.m.actions[a].kind, ActionKind::Stop) || c.holder.borrow().child {
            return Ok(Out::new());
        }
        if let Some(ci) = self.asmop_calc(c)? {
            self.calc_stop(ci, Self::op_ctx(c));
        }
        Ok(Out::new())
    }

    // --------------------------------------------------------------------------- active

    /// `onActiveResourceStateUpdated` (SS-MEAS-3.2, SS-MEAS-3.3, SS-MEAS-3.5).
    pub(super) fn mon_active_state(&mut self, ri: ResIdx, n: u64, u: f64) -> HR {
        let now = self.now;
        for k in 0..self.mon.state[ri as usize].len() {
            let s = self.mon.state[ri as usize][k];
            self.measure(s, now, n as f64);
        }
        for k in 0..self.mon.util[ri as usize].len() {
            let s = self.mon.util[ri as usize][k];
            self.measure(s, now, u);
        }
        Ok(Out::new())
    }

    /// `onResourceDemandCalculated` (SS-MEAS-3.4).
    pub(super) fn mon_active_demand(&mut self, ri: ResIdx, v: f64) -> HR {
        let now = self.now;
        for k in 0..self.mon.demand[ri as usize].len() {
            let s = self.mon.demand[ri as usize][k];
            self.measure(s, now, v);
        }
        Ok(Out::new())
    }

    // --------------------------------------------------------------------------- passive

    fn passive_probes(&self, r: &Rdr) -> Result<Option<&PassiveProbes>, Throw> {
        let Some(p) = r.passive else {
            return throw("NoSuchElementException", "No value present");
        };
        Ok(self.mon.passive.get(&*self.m.passive_resources[p].id))
    }

    fn passive_ctx(r: &Rdr) -> RequestCtx {
        RequestCtx {
            user: r.ctx.rpc.user.id,
            caller: None,
        }
    }

    /// `onResourceDemandRequest` (waiting time starts, SS-MEAS-4.2).
    pub(super) fn mon_passive_requested(&mut self, r: &Rc<Rdr>) -> HR {
        if r.active {
            return Ok(Out::new());
        }
        let Some(p) = self.passive_probes(r)? else {
            return Ok(Out::new());
        };
        let cs = p.waiting.clone();
        for ci in cs {
            self.calc_start(ci, Self::passive_ctx(r))?;
        }
        Ok(Out::new())
    }

    /// `onPassiveResourceAcquired` (waiting time stops, holding time starts).
    pub(super) fn mon_passive_acquired(&mut self, r: &Rc<Rdr>) -> HR {
        let Some(p) = self.passive_probes(r)? else {
            return Ok(Out::new());
        };
        let cs = p.acquired.clone();
        for (ci, waiting) in cs {
            if waiting {
                self.calc_stop(ci, Self::passive_ctx(r));
            } else {
                self.calc_start(ci, Self::passive_ctx(r))?;
            }
        }
        Ok(Out::new())
    }

    /// `onPassiveResourceReleased` (holding time stops).
    pub(super) fn mon_passive_released(&mut self, r: &Rc<Rdr>) -> HR {
        let Some(p) = self.passive_probes(r)? else {
            return Ok(Out::new());
        };
        let cs = p.holding.clone();
        for ci in cs {
            self.calc_stop(ci, Self::passive_ctx(r));
        }
        Ok(Out::new())
    }

    /// `onPassiveResourceStateUpdated` (available units).
    pub(super) fn mon_passive_state(&mut self, r: &Rc<Rdr>, available: i64) -> HR {
        let Some(p) = self.passive_probes(r)? else {
            return Ok(Out::new());
        };
        let ss = p.state.clone();
        let now = self.now;
        for s in ss {
            self.measure(s, now, available as f64);
        }
        Ok(Out::new())
    }
}
