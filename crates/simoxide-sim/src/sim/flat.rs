//! Execution of the flat behaviour code ([`crate::code`]): SEFF behaviours and usage-scenario
//! behaviours, including the actions that complete inside the behaviour's own continuation
//! (internal actions, external calls, entry-level system calls, delays).

use super::*;
use crate::code::{NO_TYPE, SOp, UOp};

impl<'m, C: Compat> Simulation<'m, C> {
    /// The continuation of SEFF behaviour `beh`, at its start.
    #[inline]
    pub(super) fn sbeh(&self, beh: BehaviourId) -> Cont {
        Cont::SBeh {
            pc: self.code.sbeh_start(beh),
            t0: -1,
            caller: (SYSTEM_AC, ROOT_PATH),
        }
    }

    /// The continuation of usage-scenario behaviour `beh`, at its start.
    #[inline]
    pub(super) fn ubeh(&self, beh: ScenarioBehaviourId) -> Cont {
        Cont::UBeh {
            pc: self.code.ubeh_start(beh),
            t0: -1,
            t1: -1,
        }
    }

    /// Runs the SEFF behaviour on top of the stack until it waits, pushes a child continuation
    /// or ends.
    #[inline(never)]
    pub(super) fn step_sbeh(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        let code = &self.code.sops[..];
        loop {
            let Some(Cont::SBeh { pc, t0, .. }) = d.conts.last_mut() else {
                unreachable!()
            };
            let op = code[*pc as usize];
            *pc += 1;
            match op {
                SOp::Start(start) => {
                    if let Some(t) = self.trace.as_mut() {
                        let ac = cm.ac_id(d.acs.last().map_or(SYSTEM_AC, |e| e.0));
                        let pid = self.procs[s as usize].pid;
                        let id = &cm.action_ids[start.index()];
                        t.element(self.now, true, pid, "StartAction", id, Some(ac));
                        t.element(self.now, false, pid, "StartAction", id, Some(ac));
                    }
                }
                SOp::NoStart => return self.err("RDSEFF is invalid, it misses a start action"),
                SOp::Begin(a) => self.seff_element(s, d, a, true),
                SOp::BeginRt(a, se) => {
                    self.seff_element(s, d, a, true);
                    if self.running {
                        let Some(Cont::SBeh { t0, .. }) = d.conts.last_mut() else {
                            unreachable!()
                        };
                        let rt = &mut self.procs[s as usize].rt;
                        if rt.contains(&se) {
                            return self.err(RT_SAME_CONTEXT);
                        }
                        rt.push(se);
                        *t0 = self.now;
                    }
                }
                SOp::End(a) => self.seff_element(s, d, a, false),
                SOp::EndRt(a, se) => {
                    let tstart = std::mem::replace(t0, -1);
                    if tstart >= 0 {
                        self.procs[s as usize].rt.pop();
                    }
                    self.seff_element(s, d, a, false);
                    if self.running && tstart >= 0 {
                        let now = seconds(self.now);
                        emit(
                            &mut self.meas,
                            &mut self.trace,
                            self.cfg.store_measurements,
                            self.now,
                            se,
                            now,
                            now - seconds(tstart),
                        );
                    }
                }
                SOp::Demand { a, prog, rtype } => {
                    let aid = &cm.action_ids[a.index()];
                    let cur = d.frames.last().map(|f| &**f);
                    let v = self.eval_f64(prog, cur, Origin::Elem("demand", aid))?;
                    let c = self.container_of(cur_path(d))?;
                    let rtype = (rtype != NO_TYPE).then_some(ResourceTypeId(rtype));
                    let ri = self.resource_of(c, rtype)?;
                    if self.consume(s, ri, v, 1)? {
                        return Ok(self.wait_consumed(d, ri));
                    }
                }
                SOp::Infra(a, i) => {
                    let CAct::Internal { infra, .. } = &cm.act[a.index()] else {
                        unreachable!()
                    };
                    let call = &infra[i as usize];
                    let cur = d.frames.last().map(|f| &**f);
                    let n = self.eval_i32(call.count, cur, Origin::Elem("infra", &call.id))?;
                    if let Some(t) = self.trace.as_mut() {
                        t.count(
                            self.now,
                            "infra",
                            self.procs[s as usize].pid,
                            &call.id,
                            i64::from(n),
                        );
                    }
                    // (the reference's interpreter level of the internal action stays below the
                    // calls: counted for `Limits::max_stack_depth`)
                    d.elided += 1;
                    d.conts.push(Cont::Infra {
                        a,
                        ic: i,
                        left: n,
                        caller: (SYSTEM_AC, ROOT_PATH),
                        st: 0,
                    });
                    return Ok(Flow::Continue);
                }
                SOp::ResCall(a, i) => {
                    let CAct::Internal { rescalls, .. } = &cm.act[a.index()] else {
                        unreachable!()
                    };
                    let rc = &rescalls[i as usize];
                    let cur = d.frames.last().map(|f| &**f);
                    let v = self.eval_f64(rc.count, cur, Origin::Elem("rescall", &rc.id))?;
                    let c = self.container_of(cur_path(d))?;
                    let ri = self.resource_of(c, rc.resource_type)?;
                    if self.consume(s, ri, v, rc.service_id)? {
                        return Ok(self.wait_consumed(d, ri));
                    }
                }
                SOp::Call { a, site } => {
                    let CAct::External {
                        role,
                        signature,
                        inputs,
                        ..
                    } = &cm.act[a.index()]
                    else {
                        unreachable!()
                    };
                    let Some(sig) = *signature else {
                        return self.err("external call without signature");
                    };
                    let aid = &cm.action_ids[a.index()];
                    let fin =
                        self.input_frame(inputs, d.frames.last(), Origin::Elem("param", aid))?;
                    d.frames.push(fin);
                    let c = d.acs.pop().unwrap_or((SYSTEM_AC, ROOT_PATH));
                    let Some(Cont::SBeh { caller, .. }) = d.conts.last_mut() else {
                        unreachable!()
                    };
                    *caller = c;
                    d.results.push(self.empty.clone());
                    // (the reference's interpreter level of the call)
                    d.elided += 1;
                    // the resolution depends on the caller's context and the context below it
                    let parent = d.acs.last().map_or(u32::MAX, |e| e.1);
                    let hit = &self.call_cache[site as usize];
                    if let Some(cont) = hit.cont
                        && hit.ac == c.0
                        && hit.parent == parent
                    {
                        d.conts.push(cont);
                    } else {
                        let (depth, acs) = (d.conts.len(), d.acs.len());
                        self.call_required(d, c.0, *role, sig)?;
                        // no required delegation on the way: one continuation, the same
                        // assembly-context stack
                        if d.conts.len() == depth + 1 && d.acs.len() == acs {
                            self.call_cache[site as usize] = CallCache {
                                ac: c.0,
                                parent,
                                cont: d.conts.last().copied(),
                            };
                        }
                    }
                    return self.enter_call(s, d);
                }
                SOp::Return { a, returns } => {
                    let Some(&Cont::SBeh { caller, .. }) = d.conts.last() else {
                        unreachable!()
                    };
                    d.elided -= 1;
                    d.acs.push(caller);
                    self.pool.recycle(d.frames.pop());
                    let res = d.results.pop();
                    if returns {
                        let CAct::External { returns, .. } = &cm.act[a.index()] else {
                            unreachable!()
                        };
                        let aid = &cm.action_ids[a.index()];
                        let mut target = d.frames.pop().unwrap_or_default();
                        self.pool.unshare(&mut target);
                        let r = self.fill(
                            returns,
                            res.as_ref(),
                            frame_mut(&mut target),
                            Origin::Elem("return", aid),
                        );
                        d.frames.push(target);
                        r?;
                    }
                    self.pool.recycle(res);
                }
                SOp::Exec(a) => {
                    let depth = d.conts.len();
                    match self.exec_action(s, d, a)? {
                        // done without a child continuation: go on right away
                        Flow::Continue if d.conts.len() == depth => {}
                        // a child behaviour or loop: run it right away
                        Flow::Continue => {
                            if let Some(f) = self.settle_seff(d, false)? {
                                return Ok(f);
                            }
                        }
                        f => return Ok(f),
                    }
                }
                SOp::Ret => {
                    d.conts.pop();
                    if let Some(f) = self.settle_seff(d, true)? {
                        return Ok(f);
                    }
                }
                SOp::NoStop => return self.err("NullPointerException: missing successor"),
            }
        }
    }

    /// A SEFF behaviour has pushed a child (`ended == false`) or has ended: runs the loop
    /// continuations on top and returns `None` if a SEFF behaviour is on top now (the caller
    /// goes on with it), else what `step_sbeh` returns.
    #[inline]
    fn settle_seff(&mut self, d: &mut ProcData, ended: bool) -> R<Option<Flow>> {
        loop {
            match d.conts.last_mut() {
                Some(Cont::SBeh { .. }) => {
                    self.tick()?;
                    return Ok(None);
                }
                Some(Cont::SLoop { body, left }) => {
                    self.tick()?;
                    if *left <= 0 {
                        d.conts.pop();
                    } else {
                        *left -= 1;
                        let body = *body;
                        d.conts.push(self.sbeh(body));
                    }
                }
                _ if ended => return self.resume_caller(d).map(Some),
                _ => return Ok(Some(Flow::Continue)),
            }
        }
    }

    /// [`Self::settle_seff`] for usage-scenario behaviours.
    #[inline]
    fn settle_usage(&mut self, s: u32, d: &mut ProcData, ended: bool) -> R<Option<Flow>> {
        loop {
            match d.conts.last_mut() {
                Some(Cont::UBeh { .. }) => {
                    self.tick()?;
                    return Ok(None);
                }
                Some(Cont::ULoop { body, left }) => {
                    self.tick()?;
                    if *left <= 0 {
                        d.conts.pop();
                    } else {
                        *left -= 1;
                        let body = *body;
                        d.conts.push(self.ubeh(body));
                    }
                }
                Some(Cont::Scenario { .. }) if ended => {
                    return self.enter(s, d, Self::step_scenario).map(Some);
                }
                _ => return Ok(Some(Flow::Continue)),
            }
        }
    }

    /// `BEGIN` / `END` trace line of SEFF action `a`.
    #[inline]
    fn seff_element(&mut self, s: u32, d: &ProcData, a: ActionId, begin: bool) {
        if let Some(t) = self.trace.as_mut() {
            let cm = self.cm;
            let ac = cm.ac_id(d.acs.last().map_or(SYSTEM_AC, |e| e.0));
            let pid = self.procs[s as usize].pid;
            let aid = &cm.action_ids[a.index()];
            t.element(
                self.now,
                begin,
                pid,
                cm.action_type[a.index()],
                aid,
                Some(ac),
            );
        }
    }

    /// Executes SEFF action `a` after its `BEGIN` (the actions without an instruction of their
    /// own; the behaviour continues after it when it neither waits nor pushes a child).
    fn exec_action(&mut self, s: u32, d: &mut ProcData, a: ActionId) -> R<Flow> {
        let cm = self.cm;
        let aid = &cm.action_ids[a.index()];
        let pid = self.pid(s);
        match &cm.act[a.index()] {
            CAct::Start | CAct::Stop | CAct::Internal { .. } | CAct::External { .. } => {
                unreachable!("compiled to instructions")
            }
            CAct::ProbBranch {
                cum,
                behaviours,
                sels,
            } => {
                let prev = self.rng.set_origin(Origin::Elem("branch", aid));
                let u = self.rng.next_uniform();
                self.rng.restore_origin(prev);
                let Some(i) = simoxide_random::probfn::branch_index(cum, u) else {
                    return self.err("branch: no transition selected");
                };
                if let Some(t) = self.trace.as_mut() {
                    t.branch(self.now, pid, aid, i as i64, Some(&sels[i]));
                }
                let Some(b) = behaviours[i] else {
                    return self.err("branch transition without behaviour");
                };
                d.conts.push(self.sbeh(b));
            }
            CAct::GuardBranch {
                guards,
                guard_ids,
                behaviours,
                sels,
            } => {
                let cur = d.frames.last().map(|f| &**f);
                let mut chosen = None;
                for (i, &g) in guards.iter().enumerate() {
                    if self.eval_bool(g, cur, Origin::Elem("guard", &guard_ids[i]))? {
                        chosen = Some(i);
                        break;
                    }
                }
                if let Some(t) = self.trace.as_mut() {
                    match chosen {
                        Some(i) => t.branch(self.now, pid, aid, i as i64, Some(&sels[i])),
                        None => t.branch(self.now, pid, aid, -1, None),
                    }
                }
                let Some(i) = chosen else {
                    return self.err("No branch transition was active. This is not allowed.");
                };
                let Some(b) = behaviours[i] else {
                    return self.err("branch transition without behaviour");
                };
                d.conts.push(self.sbeh(b));
            }
            CAct::EmptyBranch => return self.err("Empty branch action is not allowed"),
            CAct::Loop { count, body } => {
                let cur = d.frames.last().map(|f| &**f);
                let n = self.eval_i32(*count, cur, Origin::Elem("loop", aid))?;
                if let Some(t) = self.trace.as_mut() {
                    t.count(self.now, "loop", pid, aid, i64::from(n));
                }
                let Some(body) = *body else {
                    return self.err("loop without body");
                };
                d.conts.push(Cont::SLoop { body, left: n });
            }
            CAct::Collection { count, .. } => {
                let cur = d.frames.last().map(|f| &**f);
                let n = self.eval_i32(*count, cur, Origin::Elem("collection", aid))?;
                if let Some(t) = self.trace.as_mut() {
                    t.count(self.now, "loop", pid, aid, i64::from(n));
                }
                d.conts.push(Cont::Coll { a, left: n, st: 0 });
            }
            CAct::Fork {
                asynchronous,
                synchronous,
            } => {
                let parent = self.pkey(s);
                let frame = d.frames.last().unwrap_or(&self.empty).clone();
                let mut children = std::mem::take(&mut self.fork_scratch);
                children.clear();
                let join = match self.free_joins.pop() {
                    Some(j) => j,
                    None => {
                        self.joins.push(Vec::new());
                        (self.joins.len() - 1) as u32
                    }
                };
                for (list, sync) in [(asynchronous, false), (synchronous, true)] {
                    for &b in list {
                        let k = self.spawn(
                            PKind::Forked { sync, parent },
                            Cont::Forked { beh: b, st: 0 },
                        )?;
                        let (cs, _) = unkey(k);
                        let cd = &mut self.procs[cs as usize].data;
                        cd.frames.push(frame.clone());
                        cd.acs.extend_from_slice(&d.acs);
                        children.push(k);
                        if sync {
                            self.joins[join as usize].push(k);
                        }
                    }
                }
                if let Some(t) = self.trace.as_mut() {
                    t.fork(self.now, pid, aid, asynchronous.len(), synchronous.len());
                }
                // ForkExecutor.run: schedule all children, then join the synchronous ones
                d.conts.push(Cont::ForkJoin { a, join });
                // the parent is running: while stopped, children run synchronously here
                *self.procs[s as usize].data = std::mem::take(d);
                let mut r = Ok(());
                for &k in &children {
                    r = self.activate(k);
                    if r.is_err() {
                        break;
                    }
                }
                self.fork_scratch = children;
                *d = std::mem::take(&mut *self.procs[s as usize].data);
                r?;
            }
            CAct::Acquire(pr) => {
                if self.acquire(s, d, *pr)? {
                    return Ok(Flow::Wait);
                }
            }
            CAct::Release(pr) => {
                self.release(s, d, *pr)?;
            }
            CAct::SetVariable(usages) => {
                let cur = d.frames.last();
                let Some(mut target) = d.results.pop() else {
                    return self.err("NullPointerException: no result frame");
                };
                self.pool.unshare(&mut target);
                let r = self.fill(
                    usages,
                    cur,
                    frame_mut(&mut target),
                    Origin::Elem("setvar", aid),
                );
                d.results.push(target);
                r?;
            }
            CAct::Recovery(primary) => {
                let Some(b) = *primary else {
                    return self.err("recovery action without primary behaviour");
                };
                d.conts.push(self.sbeh(b));
            }
            CAct::Unsupported(n) => {
                return self.err(format!(
                    "SEFF Interpreter tried to interpret unsupported action type: {n}"
                ));
            }
        }
        Ok(Flow::Continue)
    }

    /// Runs the usage-scenario behaviour on top of the stack until it waits, pushes a child
    /// continuation or ends.
    #[inline(never)]
    pub(super) fn step_ubeh(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        let code = &self.code.uops[..];
        loop {
            let Some(Cont::UBeh { pc, t0, .. }) = d.conts.last_mut() else {
                unreachable!()
            };
            let op = code[*pc as usize];
            *pc += 1;
            match op {
                UOp::Start(start) => {
                    if let Some(t) = self.trace.as_mut() {
                        let pid = self.procs[s as usize].pid;
                        let id = &cm.uaction_ids[start.index()];
                        t.element(self.now, true, pid, "Start", id, None);
                        t.element(self.now, false, pid, "Start", id, None);
                    }
                }
                UOp::NoStart => {
                    return self.err("usage scenario behaviour misses a start action");
                }
                UOp::Begin(a) => self.user_element(s, a, true),
                UOp::BeginRt(a) => {
                    if self.running {
                        *t0 = self.now;
                    }
                    self.user_element(s, a, true);
                }
                UOp::End(a) => self.user_element(s, a, false),
                UOp::EndRt(a, se) => {
                    let tstart = std::mem::replace(t0, -1);
                    self.user_element(s, a, false);
                    if self.running && tstart >= 0 {
                        let now = seconds(self.now);
                        emit(
                            &mut self.meas,
                            &mut self.trace,
                            self.cfg.store_measurements,
                            self.now,
                            se,
                            now,
                            now - seconds(tstart),
                        );
                    }
                }
                UOp::Delay(a, prog) => {
                    let aid = &cm.uaction_ids[a.index()];
                    let dl = self.eval_f64(prog, None, Origin::Elem("delay", aid))?;
                    if self.hold(s, dl)? {
                        return Ok(Flow::Wait);
                    }
                }
                UOp::Call(a) => {
                    let CUAct::Elsc {
                        role,
                        signature,
                        inputs,
                        sysop_series,
                        ..
                    } = &cm.uact[a.index()]
                    else {
                        unreachable!()
                    };
                    let (Some(role), Some(sig)) = (*role, *signature) else {
                        return self.err("entry level system call without role or signature");
                    };
                    let pid = self.pid(s);
                    if let Some(t) = self.trace.as_mut() {
                        t.system_op(
                            self.now,
                            true,
                            pid,
                            &cm.role_ids[role.index()],
                            &cm.sig_ids[sig.index()],
                        );
                    }
                    if sysop_series.is_some() && self.running {
                        let Some(Cont::UBeh { t1, .. }) = d.conts.last_mut() else {
                            unreachable!()
                        };
                        *t1 = self.now;
                    }
                    let aid = &cm.uaction_ids[a.index()];
                    let fin =
                        self.input_frame(inputs, d.frames.last(), Origin::Elem("param", aid))?;
                    d.frames.push(fin);
                    d.results.push(self.empty.clone());
                    let prov = match self.elsc_prov[a.index()] {
                        u32::MAX => {
                            let p = self.prov_id(SYSTEM_AC, role, sig);
                            self.elsc_prov[a.index()] = p;
                            p
                        }
                        p => p,
                    };
                    // (the reference's interpreter level of the call)
                    d.elided += 1;
                    d.conts.push(Cont::ProvRole {
                        prov,
                        t0: -1,
                        st: 0,
                    });
                    return self.enter(s, d, Self::step_prov_role);
                }
                UOp::Return(a) => {
                    let Some(Cont::UBeh { t1, .. }) = d.conts.last_mut() else {
                        unreachable!()
                    };
                    let tstart = std::mem::replace(t1, -1);
                    let CUAct::Elsc {
                        role,
                        signature,
                        outputs,
                        sysop_series,
                        ..
                    } = &cm.uact[a.index()]
                    else {
                        unreachable!()
                    };
                    let (Some(role), Some(sig)) = (*role, *signature) else {
                        unreachable!("checked by the call")
                    };
                    d.elided -= 1;
                    self.pool.recycle(d.frames.pop());
                    let res = d.results.pop();
                    if !outputs.is_empty() {
                        let aid = &cm.uaction_ids[a.index()];
                        let mut target = d.frames.pop().unwrap_or_default();
                        self.pool.unshare(&mut target);
                        let r = self.fill(
                            outputs,
                            res.as_ref(),
                            frame_mut(&mut target),
                            Origin::Elem("return", aid),
                        );
                        d.frames.push(target);
                        r?;
                    }
                    self.pool.recycle(res);
                    let pid = self.pid(s);
                    if let Some(t) = self.trace.as_mut() {
                        t.system_op(
                            self.now,
                            false,
                            pid,
                            &cm.role_ids[role.index()],
                            &cm.sig_ids[sig.index()],
                        );
                    }
                    if let Some(se) = sysop_series
                        && self.running
                        && tstart >= 0
                    {
                        let now = seconds(self.now);
                        emit(
                            &mut self.meas,
                            &mut self.trace,
                            self.cfg.store_measurements,
                            self.now,
                            *se,
                            now,
                            now - seconds(tstart),
                        );
                    }
                }
                UOp::Exec(a) => {
                    let pid = self.pid(s);
                    let aid = &cm.uaction_ids[a.index()];
                    match &cm.uact[a.index()] {
                        CUAct::Branch {
                            cum,
                            behaviours,
                            sels,
                        } => {
                            let prev = self.rng.set_origin(Origin::Elem("branch", aid));
                            let idx = if cum.is_empty() {
                                None
                            } else {
                                simoxide_random::probfn::branch_index(cum, self.rng.next_uniform())
                            };
                            self.rng.restore_origin(prev);
                            let Some(i) = idx else {
                                return self.err("branch: no transition selected");
                            };
                            if let Some(t) = self.trace.as_mut() {
                                t.branch(self.now, pid, aid, i as i64, Some(&sels[i]));
                            }
                            let Some(b) = behaviours[i] else {
                                return self.err("branch transition without behaviour");
                            };
                            d.conts.push(self.ubeh(b));
                        }
                        CUAct::Loop { count, body } => {
                            let n = self.eval_i32(*count, None, Origin::Elem("loop", aid))?;
                            if let Some(t) = self.trace.as_mut() {
                                t.count(self.now, "loop", pid, aid, i64::from(n));
                            }
                            let Some(body) = *body else {
                                return self.err("loop without body");
                            };
                            d.conts.push(Cont::ULoop { body, left: n });
                        }
                        _ => unreachable!("compiled to instructions"),
                    }
                    if let Some(f) = self.settle_usage(s, d, false)? {
                        return Ok(f);
                    }
                }
                UOp::Ret => {
                    d.conts.pop();
                    if let Some(f) = self.settle_usage(s, d, true)? {
                        return Ok(f);
                    }
                }
                UOp::NoStop => return self.err("NullPointerException: missing successor"),
            }
        }
    }

    /// `BEGIN` / `END` trace line of user action `a`.
    #[inline]
    fn user_element(&mut self, s: u32, a: UserActionId, begin: bool) {
        if let Some(t) = self.trace.as_mut() {
            let cm = self.cm;
            let pid = self.procs[s as usize].pid;
            let aid = &cm.uaction_ids[a.index()];
            t.element(self.now, begin, pid, cm.uaction_type[a.index()], aid, None);
        }
    }
}
