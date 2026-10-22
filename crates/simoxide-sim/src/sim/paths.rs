//! Opt-in path monitoring (`docs/spec/measurements.md` MEAS-13; not part of SimuLizar):
//!
//! * **Synchronous paths** (`SimConfig::sync_paths`): per *session* (one run of a usage scenario
//!   by one user: an open-workload user's run, one iteration of a closed-workload user), the set
//!   of resource containers that hold the assembly contexts whose SEFFs executed on the
//!   session's synchronous path. The path is the user's own process and, recursively, the
//!   synchronous forked behaviours of a `ForkAction`; an asynchronous forked behaviour, and
//!   everything it starts, is off the path. A container is the one the assembly context is
//!   allocated to when its SEFF is entered (MEAS-13.1). The run keeps, per usage scenario, the
//!   number of sessions per distinct container set ([`RunState::sync_paths`]).
//! * **Call counts** (`SimConfig::call_counts`): the number of executions of every
//!   `ExternalCallAction` (counted when the call starts, on any process, synchronous or not;
//!   [`RunState::call_counts`]).
//!
//! Both only read the interpreter's state: they draw no random numbers and schedule no events,
//! so a run with them is the run without them (same events, same measurements). Off (the
//! default) they cost one predictable branch per SEFF entry, external call, process creation
//! and scenario run. Sessions count when they end while the experiment runs (as response
//! times, MEAS-3); what ended before a time is read by pausing there ([`Simulation::run_until`])
//! and subtracting.

use super::*;

/// No session ([`PathRec::owner`]).
const NONE: u64 = u64::MAX;

/// The path-monitoring state of a run (`RunState::paths`).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct PathRec {
    /// Synchronous paths are recorded.
    pub(crate) sync: bool,
    /// Per process slot: the key of the user process whose session the process serves
    /// synchronously ([`NONE`]: none; a user in a session: its own key).
    pub(crate) owner: Vec<u64>,
    /// Per process slot (users in a session): the containers of the session so far.
    pub(crate) set: Vec<Vec<u32>>,
    /// (scenario index, container indices in index order) → finished sessions.
    pub(crate) sessions: Vec<((u32, Vec<u32>), u64)>,
    /// Index into `sessions`, by key (rebuilt after a snapshot: not serialised).
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub(crate) index: FxMap<(u32, Vec<u32>), u32>,
    /// Call counts are kept.
    pub(crate) calls_on: bool,
    /// Per `ExternalCallAction` (index of its `ActionId`): executions; grown on demand.
    pub(crate) calls: Vec<u64>,
}

/// The sessions of one usage scenario with one container set ([`RunState::sync_paths`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPaths {
    /// The usage scenario (index of `CompiledModel::scenarios`) and its id.
    pub scenario: u32,
    pub scenario_id: String,
    /// The containers of the sessions' synchronous paths (sorted by id).
    pub containers: Vec<ContainerId>,
    /// Finished sessions with exactly this set.
    pub sessions: u64,
}

impl PathRec {
    pub(crate) fn new(sync: bool, calls: bool) -> Option<Box<PathRec>> {
        (sync || calls).then(|| {
            Box::new(PathRec {
                sync,
                calls_on: calls,
                ..PathRec::default()
            })
        })
    }

    fn slot(&mut self, s: u32) {
        let i = s as usize;
        if self.owner.len() <= i {
            self.owner.resize(i + 1, NONE);
            self.set.resize_with(i + 1, Vec::new);
        }
    }

    /// Rebuilds the index of `sessions` (after a snapshot was read).
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub(crate) fn reindex(&mut self) {
        self.index = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, (k, _))| (k.clone(), i as u32))
            .collect();
    }
}

impl<'m, C: Compat, F: Flavour> Simulation<'m, C, F> {
    /// Process `slot` was created with `kind` (`parent`'s session, for a synchronous fork).
    #[cold]
    #[inline(never)]
    pub(super) fn paths_spawned(&mut self, slot: u32, kind: PKind) {
        let owner = match kind {
            PKind::Forked { sync: true, parent } => {
                let (ps, _) = unkey(parent);
                self.st
                    .paths
                    .as_deref()
                    .and_then(|p| p.owner.get(ps as usize).copied())
                    .unwrap_or(NONE)
            }
            _ => NONE,
        };
        if let Some(p) = self.st.paths.as_deref_mut()
            && p.sync
        {
            p.slot(slot);
            p.owner[slot as usize] = owner;
            p.set[slot as usize].clear();
        }
    }

    /// User process `s` starts a session (a usage-scenario run).
    #[cold]
    #[inline(never)]
    pub(super) fn paths_session_start(&mut self, s: u32) {
        let k = self.pkey(s);
        if let Some(p) = self.st.paths.as_deref_mut()
            && p.sync
        {
            p.slot(s);
            p.owner[s as usize] = k;
            p.set[s as usize].clear();
        }
    }

    /// User process `s` ends its session of scenario `scen`: counted if the experiment runs.
    #[cold]
    #[inline(never)]
    pub(super) fn paths_session_end(&mut self, s: u32, scen: u32) {
        let k = self.pkey(s);
        let running = self.st.running;
        let Some(p) = self.st.paths.as_deref_mut() else {
            return;
        };
        if !p.sync || p.owner.get(s as usize) != Some(&k) {
            return;
        }
        p.owner[s as usize] = NONE;
        let mut set = std::mem::take(&mut p.set[s as usize]);
        if !running {
            set.clear();
            p.set[s as usize] = set;
            return;
        }
        set.sort_unstable();
        let key = (scen, set);
        match p.index.get(&key) {
            Some(&i) => p.sessions[i as usize].1 += 1,
            None => {
                p.index.insert(key.clone(), p.sessions.len() as u32);
                p.sessions.push((key, 1));
            }
        }
    }

    /// Process `s` enters a SEFF on assembly-context path `path`: its container joins the
    /// session the process serves synchronously, if any.
    #[cold]
    #[inline(never)]
    pub(super) fn paths_seff(&mut self, s: u32, path: u32) {
        let Some(c) = self.path_node(path).container else {
            return;
        };
        let Some(p) = self.st.paths.as_deref_mut() else {
            return;
        };
        if !p.sync {
            return;
        }
        let Some(&o) = p.owner.get(s as usize) else {
            return;
        };
        if o == NONE {
            return;
        }
        let (os, generation) = unkey(o);
        if self.st.procs.get(os as usize).map(|x| x.generation) != Some(generation) {
            return;
        }
        let set = &mut p.set[os as usize];
        if !set.contains(&c.0) {
            set.push(c.0);
        }
    }

    /// External call `a` starts.
    #[cold]
    #[inline(never)]
    pub(super) fn paths_call(&mut self, a: ActionId) {
        if let Some(p) = self.st.paths.as_deref_mut()
            && p.calls_on
        {
            let i = a.index();
            if p.calls.len() <= i {
                p.calls.resize(i + 1, 0);
            }
            p.calls[i] += 1;
        }
    }
}

impl<C: Compat, F: Flavour> RunState<C, F> {
    /// The finished sessions per usage scenario and container set (`SimConfig::sync_paths`;
    /// `None` when off), ordered by scenario index, then by container ids.
    pub fn sync_paths(&self, cm: &CompiledModel) -> Option<Vec<SessionPaths>> {
        let p = self.paths.as_deref().filter(|p| p.sync)?;
        let ids = |set: &[ContainerId]| -> Vec<&str> {
            set.iter().map(|&c| &*cm.model.containers[c].id).collect()
        };
        let v: Vec<&((u32, Vec<u32>), u64)> = p.sessions.iter().collect();
        let mut out: Vec<SessionPaths> = v
            .into_iter()
            .map(|((scen, set), n)| {
                let mut containers: Vec<ContainerId> =
                    set.iter().map(|&c| ContainerId(c)).collect();
                containers
                    .sort_by(|a, b| cm.model.containers[*a].id.cmp(&cm.model.containers[*b].id));
                SessionPaths {
                    scenario: *scen,
                    scenario_id: cm.scenario_ids[cm.scenarios[*scen as usize].id.index()]
                        .to_string(),
                    containers,
                    sessions: *n,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            a.scenario
                .cmp(&b.scenario)
                .then_with(|| ids(&a.containers).cmp(&ids(&b.containers)))
        });
        Some(out)
    }

    /// Executions of every `ExternalCallAction` so far, by `ActionId` index (shorter than the
    /// action table when the last actions never ran; `SimConfig::call_counts`, `None` when off).
    pub fn call_counts(&self) -> Option<&[u64]> {
        self.paths
            .as_deref()
            .filter(|p| p.calls_on)
            .map(|p| p.calls.as_slice())
    }
}
