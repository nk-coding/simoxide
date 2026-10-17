//! Host hooks for layers above the engine (the SPD interpreter, `simoxide-spd`): *external
//! notes* and *observed series*.
//!
//! * An external note ([`Simulation::schedule_external`]) is an ordinary note of the event list
//!   (FIFO among same-time notes, SIM-3) that does nothing in the engine: its dispatch records
//!   the note's tag. A driver that runs with [`Simulation::run_while`]`(|s| !s.hooks_pending())`
//!   gets control right after that event, before the next one, takes the tags
//!   ([`Simulation::take_external`]) and acts at the note's time.
//! * An observed series ([`Simulation::observe_series`]) copies each tuple emitted to it (raw
//!   tuples and sliding-window outputs) into a buffer ([`Simulation::take_observed`]); the
//!   driver gets control after the event that emitted it.
//!
//! * A tracked resource ([`Simulation::track_busy`]) accumulates the time integral of its busy
//!   fraction (the value its overall-utilisation probe reports: busy instances / instances), so
//!   that a host can read the mean utilisation over any interval between two readings
//!   ([`Simulation::busy_integral`]) without a monitor in the model (the HPA's CPU metric,
//!   `simoxide-spd`'s `hpa`).
//!
//! All are empty and cost one predictable branch per tuple or resource state change when
//! unused: runs without hooks are unchanged (golden hashes). The buffers and integrals are part
//! of the [`RunState`]: a fork or snapshot taken between events sees them empty when the driver
//! drains them before.

use super::*;

/// A tuple of an observed series (see [`Simulation::observe_series`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservedTuple {
    pub series: SeriesId,
    /// Simulation time of the emission (ns).
    pub at_ns: i64,
    /// The tuple: point in time (s) and value.
    pub t: f64,
    pub v: f64,
}

/// Hook state of a run (in the measurement sink, so that emissions can reach it).
#[derive(Clone, Default)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct Hooks {
    /// Tags of external notes dispatched since the last [`Simulation::take_external`].
    pub(crate) due: Vec<u32>,
    /// Some series is observed.
    pub(crate) observing: bool,
    /// Per series: observed.
    pub(crate) observe: Vec<bool>,
    pub(crate) observed: Vec<ObservedTuple>,
    /// Some resource is tracked ([`Simulation::track_busy`]).
    pub(crate) busy_on: bool,
    /// Per resource (index): its busy-time integral, if tracked.
    pub(crate) busy: Vec<Option<Busy>>,
}

/// The busy-time integral of a tracked resource.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct Busy {
    /// Tracked since (s).
    pub(crate) since: f64,
    /// Last change (s) and the busy fraction since then.
    pub(crate) t: f64,
    pub(crate) frac: f64,
    /// Integral of the busy fraction from `since` to `t` (s).
    pub(crate) acc: f64,
}

/// Resource `ri`'s busy fraction changed to `frac` at `t` (s): extends its integral, if
/// tracked.
#[cold]
#[inline(never)]
pub(crate) fn busy_changed(h: &mut Hooks, ri: ResIdx, t: f64, frac: f64) {
    if let Some(Some(b)) = h.busy.get_mut(ri as usize) {
        if t > b.t {
            b.acc += b.frac * (t - b.t);
            b.t = t;
        }
        b.frac = frac;
    }
}

/// An emission to series `s`: buffered if `s` is observed.
#[cold]
#[inline(never)]
pub(crate) fn observe_tuple(h: &mut Hooks, now: i64, s: SeriesId, t: f64, v: f64) {
    if h.observe.get(s as usize).copied().unwrap_or(false) {
        h.observed.push(ObservedTuple {
            series: s,
            at_ns: now,
            t,
            v,
        });
    }
}

/// Slingshot's utilisation sample (SS-MEAS): busy cores / cores (PS), 0 or 1 otherwise.
pub(crate) fn ss_busy_fraction(policy: SchedulingPolicy, n: u64, cores: u32) -> f64 {
    let busy = match policy {
        SchedulingPolicy::ProcessorSharing => n.min(u64::from(cores)),
        _ => u64::from(n > 0),
    };
    busy as f64 / f64::from(cores)
}

impl<C: Compat, F: Flavour> RunState<C, F> {
    /// An external note was dispatched or an observed tuple emitted, and not taken yet.
    pub fn hooks_pending(&self) -> bool {
        !self.meas.hooks.due.is_empty() || !self.meas.hooks.observed.is_empty()
    }
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// Schedules an external note with `tag` at `at_ns` (not before the current time): it is
    /// queued behind the notes already scheduled for that time and, when dispatched, counts as an
    /// event and records `tag` (see the module documentation). Allowed before the start: the
    /// note is then queued before the notes of the initialisation.
    pub fn schedule_external(&mut self, at_ns: i64, tag: u32) -> Result<(), SimError> {
        // the flavour's clock (nanoseconds, or Slingshot's double seconds)
        let mut at = F::Clock::from_ns(at_ns);
        if at < self.st.now && F::Clock::to_ns(self.st.now) == at_ns {
            // the current time in nanoseconds (Slingshot's double clock rounds): now
            at = self.st.now;
        }
        if at < self.st.now {
            let now_ns = F::Clock::to_ns(self.st.now);
            return Err(SimError {
                message: format!(
                    "external note at {at_ns} ns is before the current time {now_ns} ns"
                ),
                at_ns: now_ns,
                kind: SimErrorKind::Model,
            });
        }
        self.st.meas.events.insert(at, Ev::External(tag));
        Ok(())
    }

    /// Observes series `s` (a [`CompiledModel::series`] index): every tuple emitted to it from
    /// now on is buffered for [`Simulation::take_observed`].
    pub fn observe_series(&mut self, s: SeriesId) {
        let h = &mut self.st.meas.hooks;
        if h.observe.len() <= s as usize {
            h.observe.resize(s as usize + 1, false);
        }
        h.observe[s as usize] = true;
        h.observing = true;
    }

    /// Tracks resource `ri` (a [`CompiledModel::resources`] index of the run's current model)
    /// from now on: the time integral of its busy fraction, as its overall-utilisation probe
    /// would report it (SimuLizar: busy instances / instances; Slingshot: busy cores / cores),
    /// read with [`Simulation::busy_integral`]. Tracking a tracked resource changes nothing.
    pub fn track_busy(&mut self, ri: ResIdx) {
        let i = ri as usize;
        if i >= self.cm.resources.len() {
            return;
        }
        let h = &self.st.meas.hooks;
        if h.busy.get(i).is_some_and(Option::is_some) {
            return;
        }
        let frac = self.busy_fraction_now(ri);
        let t = F::Clock::secs(self.st.now);
        let h = &mut self.st.meas.hooks;
        if h.busy.len() <= i {
            h.busy.resize(i + 1, None);
        }
        h.busy[i] = Some(Busy {
            since: t,
            t,
            frac,
            acc: 0.0,
        });
        h.busy_on = true;
    }

    /// The tracked resource `ri`'s busy-time integral: `(since, integral)`, the time tracking
    /// started (s) and the integral of its busy fraction from then to now (s). `None` if it is
    /// not tracked.
    pub fn busy_integral(&self, ri: ResIdx) -> Option<(f64, f64)> {
        let b = (*self.st.meas.hooks.busy.get(ri as usize)?)?;
        let now = F::Clock::secs(self.st.now);
        Some((b.since, b.acc + b.frac * (now - b.t).max(0.0)))
    }

    /// The current busy fraction of resource `ri` (its overall-utilisation probe's value).
    fn busy_fraction_now(&self, ri: ResIdx) -> f64 {
        let def = &self.cm.resources[ri as usize];
        let rt = &self.st.res[ri as usize];
        if F::SLINGSHOT {
            let (n, cores) = match &rt.sched {
                Some(s) => (s.jobs(), s.cores()),
                None => (rt.delay_set.len() as u64, 1),
            };
            ss_busy_fraction(def.policy, n, cores)
        } else {
            super::busy_fraction(
                &rt.shadow,
                def.instances,
                def.policy == SchedulingPolicy::ProcessorSharing,
            )
        }
    }

    /// The tags of the external notes dispatched since the last call, in dispatch order.
    pub fn take_external(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.st.meas.hooks.due)
    }

    /// The observed tuples emitted since the last call, in emission order.
    pub fn take_observed(&mut self) -> Vec<ObservedTuple> {
        let mut v = std::mem::take(&mut self.st.meas.hooks.observed);
        if F::SLINGSHOT {
            // the buffer holds the flavour's clock values
            for o in &mut v {
                o.at_ns = F::Clock::to_ns(o.at_ns);
            }
        }
        v
    }
}
