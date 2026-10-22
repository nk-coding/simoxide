//! Removed containers and resources (`Removal`, `docs/spec/model-changes.md` MC-8) and aborted
//! requests.
//!
//! A runtime model change that removes a container (or a processing resource) from the
//! resource environment hands its resources to the run's removal policy
//! ([`crate::ChangeOptions::removal`]): they keep simulating (SimuLizar), drain, are aborted, or
//! follow Slingshot's quirk Q5. The policy lives in the resource's [`Life`]; the hot path only
//! checks `Life::closed` where a demand is issued.

use super::*;
use crate::change::Removal;

/// The life of a resource after model changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub(super) enum Life {
    /// In the resource environment.
    Live,
    /// Removed under [`Removal::Keep`]: keeps simulating.
    Kept,
    /// Removed under [`Removal::Drain`] when the model reached generation `since`: requests of
    /// earlier generations ([`Proc::epoch`] `< since`) are still served.
    Draining(u32),
    /// Removed under [`Removal::SlingshotQ5`]: the next completion aborts its job's request,
    /// the other jobs are lost.
    Q5,
    /// Gone: every demand aborts its request; no final deactivation.
    Gone,
}

impl Life {
    /// The resource no longer accepts every demand (checked where a demand is issued).
    #[inline]
    pub(super) fn closed(self) -> bool {
        !matches!(self, Life::Live | Life::Kept)
    }
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// A demand of the running process `s` on resource `ri`, which is closed: `None` if it is
    /// served (a draining resource and a request in progress at its removal), else the request
    /// is aborted ([`Self::abort_request`]).
    #[cold]
    #[inline(never)]
    pub(super) fn closed_demand(&mut self, s: u32, d: &ProcData, ri: ResIdx) -> R<Option<Flow>> {
        if let Life::Draining(since) = self.st.res[ri as usize].life
            && self.st.procs[s as usize].epoch < since
        {
            return Ok(None);
        }
        self.abort_request(s, d).map(Some)
    }

    /// Aborts the request of the running process `s` (its demand cannot be served): a closed
    /// workload user is replaced by a new one that thinks first (as Slingshot's `UserAborted`,
    /// SS-WL-5.1), an open-workload user is dropped, a forked behaviour ends (a synchronous
    /// fork's parent continues). The interrupted request records no response time; what it
    /// holds stays held. Returns the flow that ends the process.
    pub(super) fn abort_request(&mut self, s: u32, d: &ProcData) -> R<Flow> {
        let kind = self.st.procs[s as usize].kind;
        self.after_abort(kind, d)?;
        Ok(Flow::Finished)
    }

    /// What follows the abort of a process of `kind` with stacks `d`. The aborted request
    /// counts as a finished usage-scenario run, as a failed run does in SimuCom (the users'
    /// `finally { increaseMainMeasurementsCount() }`); without that a run that stops after a
    /// number of runs would never stop once every request aborts. Slingshot's quirk Q5 keeps
    /// Slingshot's `UserAborted`, which counts nothing.
    fn after_abort(&mut self, kind: PKind, d: &ProcData) -> R<()> {
        if matches!(kind, PKind::ClosedUser | PKind::OpenUser)
            && self.st.changes.removal != Removal::SlingshotQ5
            && self.st.running
        {
            self.st.main_count += 1;
        }
        match kind {
            PKind::ClosedUser => {
                let Some(&Cont::ClosedUser { scen, .. }) = d.conts.first() else {
                    return self.err("aborted closed-workload user without a workload driver");
                };
                let k = self.spawn(PKind::ClosedUser, Cont::ClosedUser { scen, st: 3 })?;
                self.activate(k)?;
            }
            PKind::Forked { sync, parent } => {
                if sync && self.alive(parent) && !self.fork_done_of(parent) && self.st.running {
                    self.resume_parent(parent)?;
                }
            }
            PKind::OpenUser | PKind::OpenGen => {}
        }
        Ok(())
    }

    /// Aborts the request of the suspended process `k` (between events, or at an event of a
    /// resource): see [`Self::abort_request`].
    pub(super) fn abort_suspended(&mut self, k: u64) -> R<()> {
        let (s, g) = unkey(k);
        let Some(p) = self.st.procs.get(s as usize) else {
            return Ok(());
        };
        if p.generation != g || p.state != PState::Suspended {
            return Ok(());
        }
        let kind = p.kind;
        let spare = self.st.spare_data.pop().unwrap_or_default();
        let data = std::mem::replace(&mut self.st.procs[s as usize].data, spare);
        self.after_abort(kind, &data)?;
        self.terminate(s, data);
        Ok(())
    }

    /// Hands the resources `removed` (which just left the resource environment) to `policy`
    /// (between events; generation `gen`: the model's generation after the change).
    pub(super) fn remove_resources(
        &mut self,
        removed: &[ResIdx],
        policy: Removal,
        gen_after: u32,
    ) -> R<()> {
        for &ri in removed {
            let rt = &mut self.st.res[ri as usize];
            if rt.life != Life::Live {
                continue;
            }
            match policy {
                Removal::Keep => rt.life = Life::Kept,
                Removal::Drain => {
                    rt.life = Life::Draining(gen_after);
                    self.retire_if_drained(ri);
                }
                Removal::SlingshotQ5 => {
                    rt.life = if rt.sched.as_ref().is_some_and(|x| x.jobs() > 0)
                        || !rt.delay_set.is_empty()
                    {
                        Life::Q5
                    } else {
                        Life::Gone
                    };
                }
                Removal::Abort => {
                    rt.life = Life::Gone;
                    let jobs = match rt.sched.as_mut() {
                        Some(sch) => {
                            let jobs = sch.job_list();
                            let def = &self.cm.resources[ri as usize];
                            *sch =
                                ResRt::<F::Clock>::new(def, self.st.cfg.ps_algorithm, F::SLINGSHOT)
                                    .sched
                                    .expect("scheduler");
                            jobs
                        }
                        None => std::mem::take(&mut rt.delay_set),
                    };
                    for x in rt.shadow.iter_mut() {
                        *x = 0;
                    }
                    self.st.meas.events.clear_timer(ri);
                    for k in jobs {
                        self.abort_suspended(k)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A draining resource whose queue is empty and which no request of an earlier generation
    /// can reach any more is gone.
    pub(super) fn retire_if_drained(&mut self, ri: ResIdx) {
        let rt = &self.st.res[ri as usize];
        let Life::Draining(since) = rt.life else {
            return;
        };
        let busy = match &rt.sched {
            Some(s) => s.jobs() > 0,
            None => !rt.delay_set.is_empty(),
        };
        if busy {
            return;
        }
        let in_flight = self
            .st
            .procs
            .iter()
            .any(|p| p.state != PState::Terminated && p.epoch < since);
        if !in_flight {
            self.st.res[ri as usize].life = Life::Gone;
        }
    }

    /// The pending completion of resource `ri`, removed under Slingshot's Q5: its job's request
    /// is aborted, the other jobs are lost (their processes never continue); no samples.
    #[cold]
    #[inline(never)]
    pub(super) fn q5_wake(&mut self, ri: ResIdx) -> R<()> {
        let rt = &mut self.st.res[ri as usize];
        let job = rt.sched.as_mut().and_then(|sch| {
            let w = sch.pending_wakeup()?;
            sch.on_wakeup(self.st.now, &w, &mut ())
                .ok()
                .flatten()
                .map(|c| c.job)
        });
        let def = &self.cm.resources[ri as usize];
        rt.sched = ResRt::<F::Clock>::new(def, self.st.cfg.ps_algorithm, F::SLINGSHOT).sched;
        for x in rt.shadow.iter_mut() {
            *x = 0;
        }
        rt.life = Life::Gone;
        self.st.meas.events.clear_timer(ri);
        if let Some(k) = job {
            self.abort_suspended(k)?;
        }
        Ok(())
    }

    /// The end of a delay demand on resource `ri`, removed under Slingshot's Q5: every job of a
    /// delay resource has its own pending event, so each is aborted at its end.
    #[cold]
    #[inline(never)]
    pub(super) fn q5_delay_end(&mut self, ri: ResIdx, k: u64) -> R<()> {
        let rt = &mut self.st.res[ri as usize];
        let Some(pos) = rt.delay_set.iter().position(|&x| x == k) else {
            return Ok(());
        };
        rt.delay_set.swap_remove(pos);
        if rt.delay_set.is_empty() {
            rt.life = Life::Gone;
        }
        self.abort_suspended(k)
    }

    /// Whether the final deactivation (and its tuples) includes resource `ri`.
    pub(super) fn deactivates(&mut self, ri: ResIdx) -> bool {
        self.retire_if_drained(ri);
        !matches!(self.st.res[ri as usize].life, Life::Gone)
    }
}
