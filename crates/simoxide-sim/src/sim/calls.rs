//! Calls between components (provided roles, assembly connectors) and the direct hand-over of
//! control between continuations.
//!
//! A continuation that pushes a child or ends does not return to [`Simulation::run_process`]
//! when the next continuation is known: it calls that continuation's handler directly
//! ([`Simulation::enter`]), which saves the dispatch on the continuation kind. The nesting of
//! such direct calls is bounded by [`MAX_DIRECT`]; beyond it, control returns to
//! `run_process`, which dispatches as usual. Either way the continuations run in the same
//! order, so the results do not depend on it.

use super::*;

/// Maximum nesting of direct handler calls ([`Simulation::enter`]).
const MAX_DIRECT: u32 = 24;

/// A continuation handler.
type Handler<S> = fn(&mut S, u32, &mut ProcData) -> R<Flow>;

impl<'m, C: Compat> Simulation<'m, C> {
    /// Runs handler `f` of the continuation on top of the stack right away (one interpreter
    /// step), unless the direct calls are nested too deeply already.
    #[inline(always)]
    pub(super) fn enter(&mut self, s: u32, d: &mut ProcData, f: Handler<Self>) -> R<Flow> {
        if self.direct >= MAX_DIRECT {
            return Ok(Flow::Continue);
        }
        self.tick()?;
        self.direct += 1;
        let r = f(self, s, d);
        self.direct -= 1;
        r
    }

    /// Counts an interpreter step that does not go through `run_process` (`Limits::max_steps`,
    /// deadline and cancellation).
    #[inline(always)]
    pub(super) fn tick(&mut self) -> R<()> {
        self.steps += 1;
        if self.steps >= self.next_step_check {
            self.check_steps()?;
        }
        Ok(())
    }

    /// The continuation on top of the stack has just ended: the required-delegation levels
    /// below it end as well (they restore the assembly-context stack); the caller goes on with
    /// the next step. (Calling the caller's handler directly as well was measured: no gain.)
    #[inline]
    pub(super) fn resume_caller(&mut self, d: &mut ProcData) -> R<Flow> {
        while let Some(&Cont::ReqDeleg { ac }) = d.conts.last() {
            d.conts.pop();
            d.acs.push(ac);
        }
        Ok(Flow::Continue)
    }

    /// A call was pushed (`call_required`): run it.
    #[inline]
    pub(super) fn enter_call(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        match d.conts.last() {
            Some(Cont::AsmConn { .. }) => self.enter(s, d, Self::step_asm_conn),
            Some(Cont::ProvRole { .. }) => self.enter(s, d, Self::step_prov_role),
            _ => Ok(Flow::Continue),
        }
    }

    /// A call of a provided role: its start (stage 0) pushes the providing entity (an inner
    /// provided role or the SEFF), its end (stage 1) follows when that has ended.
    #[inline(never)]
    pub(super) fn step_prov_role(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        loop {
            let Some(&mut Cont::ProvRole {
                prov,
                ref mut t0,
                ref mut st,
            }) = d.conts.last_mut()
            else {
                unreachable!()
            };
            let pr = self.provs[prov as usize];
            let (ac, role, sig) = (pr.ac, pr.role, pr.sig);
            if *st == 0 {
                *st = 1;
                self.ac_push(&mut d.acs, ac);
                if let Some(t) = self.trace.as_mut() {
                    t.assembly_op(
                        self.now,
                        true,
                        self.procs[s as usize].pid,
                        cm.ac_id(ac),
                        &cm.role_ids[role.index()],
                        &cm.sig_ids[sig.index()],
                    );
                }
                if let Some(se) = pr.aser
                    && self.running
                {
                    let rt = &mut self.procs[s as usize].rt;
                    if rt.contains(&se) {
                        return self.err(RT_SAME_CONTEXT);
                    }
                    rt.push(se);
                    *t0 = self.now;
                }
                // the providing entity
                match pr.target {
                    ProvTarget::Fail(msg) => return self.err(msg),
                    ProvTarget::Deleg { ia, ir, inner } => {
                        let inner = if inner == u32::MAX {
                            let id = self.prov_id(ia, ir, sig);
                            if let ProvTarget::Deleg { inner, .. } =
                                &mut self.provs[prov as usize].target
                            {
                                *inner = id;
                            }
                            id
                        } else {
                            inner
                        };
                        d.conts.push(Cont::ProvRole {
                            prov: inner,
                            t0: -1,
                            st: 0,
                        });
                        // the inner call starts right away
                        self.tick()?;
                    }
                    ProvTarget::Basic { comp, seff } => {
                        // basic component (ACT-2.3)
                        let acid = AssemblyContextId(ac);
                        // Component-parameter frame (parent: the current frame). Without
                        // parameters it is empty and only ever read through, so the current
                        // frame itself stands in for it (identical lookups and contents).
                        let cparams = &cm.comp_params[comp.index()];
                        let fc = match d.frames.last() {
                            Some(cur) if cparams.is_empty() => cur.clone(),
                            cur => {
                                let mut fc = self.pool.frame(cur.cloned());
                                self.fill(cparams, cur, frame_mut(&mut fc), Origin::Plain("?"))?;
                                fc
                            }
                        };
                        let mut fa = self.pool.frame(Some(fc.clone()));
                        self.fill(
                            &cm.ac_params[acid.index()],
                            Some(&fc),
                            frame_mut(&mut fa),
                            Origin::Plain("?"),
                        )?;
                        // only calls can nest without bound (recursion); behaviours and loops
                        // are bounded by the model
                        if d.conts.len() + d.elided as usize > self.max_depth {
                            return self.limit_err(
                                SimErrorKind::Limit,
                                format!(
                                    "limit exceeded: process stack deeper than {} (Limits::max_stack_depth; unbounded recursion?)",
                                    self.max_depth
                                ),
                            );
                        }
                        d.frames.push(fc);
                        d.frames.push(fa);
                        self.component_instance(d)?;
                        // the SEFF for the signature (matched by signature id)
                        let Some(beh) = seff else {
                            return self.err("Only exactly one SEFF is currently supported.");
                        };
                        // (the reference's SEFF exit level: its frames are popped by the end of
                        // this call below)
                        d.elided += 1;
                        d.conts.push(self.sbeh(beh));
                        return self.enter(s, d, Self::step_sbeh);
                    }
                }
            } else {
                let t0 = *t0;
                if let ProvTarget::Basic { .. } = pr.target {
                    // the exit of the basic component's SEFF
                    d.elided -= 1;
                    self.pool.recycle(d.frames.pop());
                    self.pool.recycle(d.frames.pop());
                }
                d.conts.pop();
                d.acs.pop();
                if t0 >= 0 {
                    self.procs[s as usize].rt.pop();
                }
                if let Some(t) = self.trace.as_mut() {
                    t.assembly_op(
                        self.now,
                        false,
                        self.procs[s as usize].pid,
                        cm.ac_id(ac),
                        &cm.role_ids[role.index()],
                        &cm.sig_ids[sig.index()],
                    );
                }
                if let Some(se) = pr.aser
                    && self.running
                    && t0 >= 0
                {
                    let now = seconds(self.now);
                    emit(
                        &mut self.meas,
                        &mut self.trace,
                        self.cfg.store_measurements,
                        self.now,
                        se,
                        now,
                        now - seconds(t0),
                    );
                }
                // an enclosing delegation ends right away; otherwise the caller goes on
                if let Some(Cont::ProvRole { .. }) = d.conts.last() {
                    self.tick()?;
                    continue;
                }
                return self.resume_caller(d);
            }
        }
    }

    /// A call through an assembly connector: the request transmission (stage 0), the call
    /// (stage 1), the reply transmission (stage 2).
    #[inline(never)]
    pub(super) fn step_asm_conn(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        loop {
            let Some(&mut Cont::AsmConn {
                src,
                dst,
                prov,
                ref mut st,
            }) = d.conts.last_mut()
            else {
                unreachable!()
            };
            match *st {
                0 => {
                    *st = 1;
                    let payload = d.frames.last().map(|f| &**f);
                    if let Some(res) = self.transmit(s, src, dst, payload)? {
                        return Ok(self.wait_consumed(d, res));
                    }
                }
                1 => {
                    *st = 2;
                    d.conts.push(Cont::ProvRole {
                        prov,
                        t0: -1,
                        st: 0,
                    });
                    return self.enter(s, d, Self::step_prov_role);
                }
                2 => {
                    *st = 3;
                    let payload = d.results.last().map(|f| &**f);
                    if let Some(res) = self.transmit(s, dst, src, payload)? {
                        return Ok(self.wait_consumed(d, res));
                    }
                }
                _ => {
                    d.conts.pop();
                    return self.resume_caller(d);
                }
            }
        }
    }
}
