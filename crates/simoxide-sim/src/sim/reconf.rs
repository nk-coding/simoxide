//! The runtime measurement model (PRM) and the reconfiguration process (MEAS-7.2, 7.3, 7.5;
//! DESIGN-SPD R1, plan items D2 and D3).
//!
//! * **PRM values (D2).** Every PRM recorder (a measurement specification with
//!   `triggersSelfAdaptations`: feed-through, fixed- and variable-size aggregation, time-driven
//!   aggregation and utilisation windows) owns one runtime measurement and writes its value: the
//!   tuple's value, or the statistical characterisation of its buffer or window
//!   ([`crate::stats`]). The values are plain data of the run state ([`Prm`]), so forks,
//!   snapshots and resumes carry them; a host mirrors them into a QVTo PRM model when rules run.
//! * **The reconfiguration process (D3).** The `Reconfigurator` observes the PRM: a write at
//!   `now > lastReconfigurationTime` while the process is not scheduled creates the
//!   `ReconfigurationProcess` once and schedules its `Resume` at `now` (`Ev::Reconf`). Each run
//!   reconfigures and passivates. Without a host the run has no rules and succeeds (one
//!   `Reconfiguration Time` tuple per run, REF-12). With a host
//!   ([`Simulation::attach_reconfiguration`]) the run waits for it: the `Ev::Reconf` event only
//!   marks the run as due ([`RunState::reconf_pending`]), a driver that runs with
//!   [`Simulation::run_while`]`(|s| !s.reconf_pending())` gets control right after that event,
//!   runs the rules (their changes are applied between events, [`Simulation::apply_changes`])
//!   and reports SimuLizar's result with [`Simulation::reconf_done`]. Nothing else happens in
//!   the `Ev::Reconf` event, so this equals running the rules inside it. After the stop
//!   (`finalise`) a write runs the process synchronously in SimuLizar; with a host the stop
//!   pauses after the finalisation step of the write (a window flush, a resource's
//!   deactivation, the passive-resource cleanup) and the rules see the PRM values of the write.

use super::*;
use crate::stats::aggregate;
use crate::windows::Amount;

/// A runtime measurement of the run's PRM with its current value
/// ([`Simulation::prm_measurements`]).
#[derive(Clone, Copy, Debug)]
pub struct PrmMeasurement<'a> {
    pub def: &'a PrmMeas,
    /// `measuringValue` (EMF default 0.0 until the recorder writes).
    pub value: f64,
}

/// A run of the reconfiguration process that waits for its host
/// ([`Simulation::reconf_due`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReconfDue {
    /// Simulation time of the run (ns).
    pub at_ns: i64,
    /// The run comes from a PRM write after the stop (inside `finalise`).
    pub post_stop: bool,
}

/// `Reconfigurator` / `ReconfigurationProcess` state (MEAS-7.2).
#[derive(Default, Clone)]
pub(super) struct Reconf {
    pub(super) created: bool,
    /// Its `Resume` note is pending (`isScheduled()`).
    pub(super) scheduled: bool,
    /// `lastReconfigurationTime` (engine clock).
    pub(super) last_ns: i64,
    /// The experiment has stopped: scheduling resumes the process synchronously.
    pub(super) stopped: bool,
    /// `Reconfiguration Time Tuple` series (one tuple per successful run).
    pub(super) time_series: Vec<SeriesId>,
    pub(super) store: bool,
    /// A synchronous run (after the stop) is in progress.
    pub(super) running: bool,
    /// The process died: a PRM write during its own synchronous run scheduled it again.
    pub(super) error: Option<&'static str>,
    /// A host runs reconfiguration rules ([`Simulation::attach_reconfiguration`]).
    pub(super) host: bool,
    /// The run waiting for the host.
    pub(super) due: Option<Due>,
    /// `NumberOfResourceContainerTrackingListener.previousNumberOfContainers`.
    pub(super) containers: usize,
    /// Where a paused `finalise` continues.
    pub(super) fin: Option<FinStep>,
    /// The deactivation order of a paused `finalise` (taken when the deactivation starts).
    pub(super) fin_order: Vec<ResIdx>,
}

#[derive(Clone)]
pub(super) struct Due {
    at: i64,
    post_stop: bool,
    /// After the stop: `lastReconfigurationTime` before the write (restored while the run's
    /// own tuples are emitted, REF-11) and the PRM values at the write.
    prev_last: i64,
    values: Option<Vec<f64>>,
}

/// A step of the finalisation (SIM-6.5, MEAS-10.1); a host pauses between steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FinStep {
    /// Final partial window `i`.
    Windows(usize),
    /// Deactivation of the `i`-th resource of the deactivation order.
    Deactivate(usize),
    /// Processes still waiting at passive resources.
    Passive,
    Done,
}

/// PRM recorders (`triggersSelfAdaptations`, MEAS-7.2) and the values of the runtime
/// measurements (MEAS-7.5).
#[derive(Default, Clone)]
pub(super) struct Prm {
    /// Series id -> indices into `recs`.
    pub(super) series: Vec<Vec<u32>>,
    pub(super) recs: Vec<PrmState>,
    /// `measuringValue` per runtime measurement, in PRM order.
    pub(super) values: Vec<f64>,
    /// Per runtime measurement: index into [`CompiledModel::prm_meas`], or [`PASSIVE`] plus an
    /// index into [`CompiledModel::prm_passive`] (recorders of lazily created calculators).
    pub(super) list: Vec<u32>,
}

/// Marks a runtime measurement of a lazily created calculator in [`Prm::list`].
pub(super) const PASSIVE: u32 = 1 << 31;

#[derive(Clone)]
pub(super) struct PrmState {
    rec: PrmRec,
    /// Its runtime measurement (index into [`Prm::values`]).
    meas: u32,
    /// Tuples until the next aggregation check (`measurementsUntilNextAggregation`).
    until: i64,
    /// Fixed size: the circular buffer `(point in time, value)` in array order, and the slot of
    /// the eldest element once it is full.
    buf: Vec<(f64, f64)>,
    eldest: usize,
    /// Variable size: the buffered tuples, oldest first.
    deque: std::collections::VecDeque<(f64, f64)>,
    /// Aggregates `Long` values (passive-resource state): the aggregation fails.
    pub(super) long_values: bool,
}

impl PrmState {
    pub(super) fn new(rec: PrmRec, meas: u32) -> PrmState {
        let until = match rec {
            PrmRec::FeedThrough => 0,
            PrmRec::Fixed { freq, .. } | PrmRec::Variable { freq, .. } => freq,
        };
        PrmState {
            rec,
            meas,
            until,
            buf: Vec::new(),
            eldest: 0,
            deque: Default::default(),
            long_values: false,
        }
    }

    /// A tuple reaches the recorder: the value it writes into the PRM, if it writes.
    fn tuple(&mut self, t: f64, v: f64) -> Option<f64> {
        match self.rec {
            PrmRec::FeedThrough => Some(v),
            PrmRec::Fixed { freq, n, stat } => {
                // FixedSizeMeasurementsAggregator.InternalBuffer: overwrite the eldest when full
                let n = n as usize;
                if self.buf.len() < n {
                    self.buf.push((t, v));
                } else {
                    self.buf[self.eldest] = (t, v);
                    self.eldest = (self.eldest + 1) % n;
                }
                self.until -= 1;
                if self.until != 0 {
                    return None;
                }
                self.until = freq;
                if self.buf.len() != n {
                    return None;
                }
                // interval: eldest to newest; the data iterate the array in index order
                let left = Amount::of(self.buf[self.eldest].0);
                let right = Amount::of(self.buf[(self.eldest + n - 1) % n].0);
                Some(aggregate(stat, &self.buf, left, right, right.minus(&left)))
            }
            PrmRec::Variable {
                freq,
                retro,
                continuous,
                stat,
            } => {
                self.deque.push_back((t, v));
                self.until -= 1;
                if self.until != 0 {
                    return None;
                }
                self.until = freq;
                let r = Amount::of(retro);
                let (first, last) = (self.deque[0].0, self.deque[self.deque.len() - 1].0);
                // aggregationRequired: !last.minus(R).isLessThan(first)
                if Amount::of(last).minus(&r).is_less_than(&Amount::of(first)) {
                    return None;
                }
                // onPreAggregate: evict what lies more than R before the newest tuple
                let old = |x: f64| Amount::of(last).minus(&Amount::of(x)).is_greater_than(&r);
                if continuous {
                    let mut polled = None;
                    while self.deque.front().is_some_and(|&(x, _)| old(x)) {
                        polled = self.deque.pop_front();
                    }
                    if let Some(x) = polled {
                        self.deque.push_front(x);
                    }
                } else {
                    while self.deque.front().is_some_and(|&(x, _)| old(x)) {
                        self.deque.pop_front();
                    }
                }
                // interval: [max(0, last - R), last]
                let right = Amount::of(last);
                let mut left = right.minus(&r);
                if left.is_less_than(&Amount::exact(0)) {
                    left = Amount::exact(0);
                }
                let data = self.deque.make_contiguous();
                Some(aggregate(stat, data, left, right, right.minus(&left)))
            }
        }
    }
}

/// A PRM write at `now` (`Reconfigurator.checkAndExecuteReconfigurations`).
#[cold]
#[inline(never)]
pub(super) fn prm_write(meas: &mut Sink, trace: &mut Option<TraceOut>, now: i64) {
    let r = &mut meas.reconf;
    if !meas.prm_armed || now <= r.last_ns || r.scheduled {
        return;
    }
    if !r.created {
        r.created = true;
        meas.next_pid += 1;
        meas.reconf_pid = meas.next_pid;
        if let Some(tr) = trace.as_mut() {
            tr.spawn(
                now,
                meas.next_pid,
                "ReconfigurationProcess",
                "Reconfiguration Process",
                meas.cur_pid,
            );
        }
    }
    if r.running {
        // a write by the process's own run (a triggering Reconfiguration Time spec) before
        // lastReconfigurationTime is updated: scheduleAt of a running process aborts the run
        r.error = Some(
            "IllegalStateException: Tried to schedule thread which was not suspended [Reconfiguration Process]",
        );
        meas.prm_armed = false;
        if let Some(tr) = trace.as_mut() {
            tr.pend(now, meas.reconf_pid);
        }
        return;
    }
    if r.stopped {
        // scheduleAt(0) of a stopped experiment resumes the process right away (and
        // lastReconfigurationTime is set after it returns)
        if r.host {
            // the host runs the rules when the finalisation step ends, on the values of now
            let prev_last = r.last_ns;
            r.last_ns = now;
            r.due = Some(Due {
                at: now,
                post_stop: true,
                prev_last,
                values: Some(meas.prm.values.clone()),
            });
            return;
        }
        r.running = true;
        reconf_run(meas, trace, now);
        meas.reconf.running = false;
        meas.reconf.last_ns = now;
    } else {
        r.last_ns = now;
        r.scheduled = true;
        meas.events.insert(now, Ev::Reconf);
    }
}

/// The `Resume` of the reconfiguration process (`Ev::Reconf`): a run without rules, or, with
/// a host, the run waits for it.
pub(super) fn reconf_resume(meas: &mut Sink, trace: &mut Option<TraceOut>, now: i64) {
    meas.reconf.scheduled = false;
    if meas.reconf.host {
        meas.reconf.due = Some(Due {
            at: now,
            post_stop: false,
            prev_last: 0,
            values: None,
        });
    } else {
        reconf_run(meas, trace, now);
    }
}

/// A successful run of the reconfiguration process: its `Reconfiguration Time` tuples; the
/// reconfiguration time is 0 (simulation time does not advance during a run).
#[cold]
#[inline(never)]
pub(super) fn reconf_run(meas: &mut Sink, trace: &mut Option<TraceOut>, now: i64) {
    for i in 0..meas.reconf.time_series.len() {
        let s = meas.reconf.time_series[i];
        let store = meas.reconf.store;
        emit(meas, trace, store, now, s, seconds(now), 0.0);
    }
}

/// A tuple `(t, v)` of series `s` reaches its PRM recorders.
#[cold]
#[inline(never)]
pub(super) fn prm_tuple(
    meas: &mut Sink,
    trace: &mut Option<TraceOut>,
    now: i64,
    s: SeriesId,
    t: f64,
    v: f64,
) {
    let Some(n) = meas.prm.series.get(s as usize).map(Vec::len) else {
        return;
    };
    let mut write = false;
    for i in 0..n {
        let r = meas.prm.series[s as usize][i] as usize;
        let rec = &mut meas.prm.recs[r];
        if let Some(x) = rec.tuple(t, v) {
            if rec.long_values {
                meas.error.get_or_insert(LONG_AGGREGATION_ERROR);
            }
            let m = rec.meas as usize;
            meas.prm.values[m] = x;
            write = true;
        }
    }
    if write {
        prm_write(meas, trace, now);
    }
}

/// The number of top-level resource containers of the allocation's resource environment
/// (`NumberOfResourceContainerTrackingListener.getNumberOfResourceContainers`).
fn top_level_containers(m: &Model) -> usize {
    let env = m
        .allocations
        .iter()
        .find_map(|a| a.environment)
        .or_else(|| m.resource_environments.ids().next());
    env.map_or(0, |e| m.resource_environments[e].containers.len())
}

impl Prm {
    /// Attaches the recorders created at initialisation (the probe-framework decorators).
    pub(super) fn attach(&mut self, cm: &CompiledModel) {
        self.series = vec![Vec::new(); cm.series.len()];
        for &(s, rec, mi) in &cm.prm_series {
            self.series[s as usize].push(self.recs.len() as u32);
            self.recs.push(PrmState::new(rec, mi));
        }
        self.values = vec![0.0; cm.prm_meas.len()];
        self.list = (0..cm.prm_meas.len() as u32).collect();
    }

    /// Attaches the recorder `p` (an index into [`CompiledModel::prm_passive`]) to the series
    /// `s` of a lazily created calculator: its runtime measurement joins the PRM.
    pub(super) fn attach_passive(&mut self, s: SeriesId, p: u32, rec: PrmRec) -> &mut PrmState {
        if self.series.len() <= s as usize {
            self.series.resize(s as usize + 1, Vec::new());
        }
        self.series[s as usize].push(self.recs.len() as u32);
        let m = self.values.len() as u32;
        self.values.push(0.0);
        self.list.push(PASSIVE | p);
        self.recs.push(PrmState::new(rec, m));
        self.recs.last_mut().expect("pushed")
    }
}

impl<C: Compat, F: Flavour> RunState<C, F> {
    /// A run of the reconfiguration process waits for its host ([`Simulation::reconf_due`]).
    pub fn reconf_pending(&self) -> bool {
        self.meas.reconf.due.is_some()
    }
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// Attaches a host that runs SimuLizar's reconfiguration rules: from now on every run of
    /// the reconfiguration process waits for the host (see the module documentation). Part of
    /// the run state (forks keep it). A run without a host attached behaves as before: its
    /// reconfigurations have no rules and succeed.
    pub fn attach_reconfiguration(&mut self) {
        self.st.meas.reconf.host = true;
    }

    /// Whether a reconfiguration host is attached.
    pub fn reconfiguration_attached(&self) -> bool {
        self.st.meas.reconf.host
    }

    /// The run of the reconfiguration process that waits for the host, if any.
    pub fn reconf_due(&self) -> Option<ReconfDue> {
        self.st.meas.reconf.due.as_ref().map(|d| ReconfDue {
            at_ns: F::Clock::to_ns(d.at),
            post_stop: d.post_stop,
        })
    }

    /// The runtime measurements of the PRM in SimuLizar's order, with their values as the
    /// waiting run of the reconfiguration process sees them (after the stop: the values at the
    /// write that triggered it), otherwise the current ones.
    pub fn prm_measurements(&self) -> Vec<PrmMeasurement<'_>> {
        let prm = &self.st.meas.prm;
        let values = match self
            .st
            .meas
            .reconf
            .due
            .as_ref()
            .and_then(|d| d.values.as_ref())
        {
            Some(v) => v.as_slice(),
            None => prm.values.as_slice(),
        };
        values
            .iter()
            .zip(&prm.list)
            .map(|(&value, &i)| PrmMeasurement {
                def: if i & PASSIVE != 0 {
                    &self.cm.prm_passive[(i & !PASSIVE) as usize].meas
                } else {
                    &self.cm.prm_meas[i as usize]
                },
                value,
            })
            .collect()
    }

    /// Ends the waiting run of the reconfiguration process with the rules' result (SimuLizar's
    /// `result &= ...` over the rules: every rule applied). A successful run notifies the
    /// reconfiguration listeners in registration order: the container count (a tuple when the
    /// number of top-level containers changed, MEAS-7.3), then the `Reconfiguration Time`
    /// tuples. An error raised by these tuples (e.g. REF-11 after the stop) ends the run.
    /// Without a waiting run nothing happens.
    pub fn reconf_done(&mut self, success: bool) -> Result<(), SimError> {
        let Some(due) = self.st.meas.reconf.due.take() else {
            return Ok(());
        };
        let now = due.at;
        let store = self.st.cfg.store_measurements;
        if due.post_stop {
            // the synchronous run: lastReconfigurationTime is set when it returns
            self.st.meas.reconf.running = true;
            self.st.meas.reconf.last_ns = due.prev_last;
        }
        if success {
            if let Some((s, _)) = self.cm.container_count {
                let n = top_level_containers(&self.cm.model);
                if n != self.st.meas.reconf.containers {
                    self.st.meas.reconf.containers = n;
                    emit(
                        &mut self.st.meas,
                        &mut self.trace,
                        store,
                        now,
                        s,
                        F::Clock::secs(now),
                        n as f64,
                    );
                }
            }
            reconf_run(&mut self.st.meas, &mut self.trace, now);
        }
        if due.post_stop {
            self.st.meas.reconf.running = false;
            self.st.meas.reconf.last_ns = now;
            // errors surface when the finalisation goes on (as in `finalise`)
            return Ok(());
        }
        if let Some(e) = self.st.meas.error {
            return Err(self.fail_run(e));
        }
        Ok(())
    }
}
