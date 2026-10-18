//! The steps of a load profile in a run (`crate::load`, `docs/spec/workloads.md` LP-4 and
//! LP-5): at an `Ev::Load` event the points due now apply. An open workload's pending arrival
//! is moved so that the rest of its interval passes at the new rate; a closed workload's
//! population becomes the model's population times the new factor.

use super::*;

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// `Ev::Load`: the profile's points due at `now`.
    pub(super) fn load_step(&mut self) -> R<()> {
        if !self.st.running {
            return Ok(());
        }
        let now = self.st.now;
        let Some(lp) = self.st.lp.as_deref_mut() else {
            return Ok(());
        };
        let changed = lp.advance(now);
        for (scen, old, new) in changed {
            let cm = self.cm;
            let Some(sc) = cm.scenarios.get(scen) else {
                continue;
            };
            if sc.removed {
                continue;
            }
            match sc.workload {
                CWorkload::Open { .. } => self.rescale_arrival(scen, old, new)?,
                CWorkload::Closed { population, .. } => {
                    let to = crate::load::scaled_population(population, new);
                    self.set_population(scen as u32, to)?;
                }
            }
        }
        Ok(())
    }

    /// Moves the pending arrival of the open workload `scen` from rate factor `old` to `new`:
    /// the remaining time `r` becomes `r · old / new` (LP-2). The generator's old wake-up
    /// becomes stale: the generator gets a new generation, so the old `Think` event no longer
    /// names it.
    fn rescale_arrival(&mut self, scen: usize, old: f64, new: f64) -> R<()> {
        let now = self.st.now;
        let Some(lp) = self.st.lp.as_deref() else {
            return Ok(());
        };
        let Some(Some((slot, at))) = lp.wake.get(scen).copied() else {
            return Ok(());
        };
        if at <= now {
            // the arrival is due now: it draws its next interval at the new rate
            return Ok(());
        }
        let p = &self.st.procs[slot as usize];
        let is_gen = p.state != PState::Terminated
            && p.in_think
            && p.kind == PKind::OpenGen
            && matches!(p.data.conts.first(), Some(Cont::OpenGen { scen: x, .. }) if *x as usize == scen);
        if !is_gen {
            return Ok(());
        }
        let rest = F::Clock::secs(at) - F::Clock::secs(now);
        let Some(to) = F::Clock::checked_after(now, rest * old / new) else {
            return self.err(format!(
                "load profile: invalid rescaled arrival time {rest}"
            ));
        };
        let p = &mut self.st.procs[slot as usize];
        p.generation = p.generation.wrapping_add(1);
        let k = key(slot, p.generation);
        self.st.meas.events.insert(to, Ev::Think(k));
        if let Some(lp) = self.st.lp.as_deref_mut() {
            lp.wake[scen] = Some((slot, to));
        }
        Ok(())
    }

    /// A closed workload's population becomes `to`: new users start now, surplus users leave
    /// after their current run, oldest first (`ClosedWorkload.setPopulation`, MC-5.2).
    pub(super) fn set_population(&mut self, scen: u32, to: i64) -> R<()> {
        let users = self.closed_users(scen);
        let n = users.len() as i64;
        let to = to.max(0);
        if n > to {
            // `ClosedWorkload.stopUsers`: the oldest users leave after their run
            for &k in &users[..(n - to) as usize] {
                self.st.procs[unkey(k).0 as usize].ss_flags |= STOP_REQUESTED;
            }
        }
        for _ in n..to {
            let k = self.spawn(PKind::ClosedUser, Cont::ClosedUser { scen, st: 0 })?;
            self.activate(k)?;
        }
        Ok(())
    }
}
