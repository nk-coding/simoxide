//! Usage streams (`SimConfig::usage_streams`, `docs/spec/random.md` RND-11): common random
//! numbers across models that share a usage model.
//!
//! With one stream per run (RND-1), two models that differ only in their system (another
//! component, another demand, another allocation) draw a different number of uniforms between
//! two users, so the users of the two runs arrive at different times and take different paths
//! after the first few events. With usage streams, draws are made from streams per source:
//!
//! * each usage scenario has an **arrival stream** for its open workload's inter-arrival times,
//!   seeded from the run seed and the scenario's id (the generator process draws from it);
//! * each user (an open-workload user, or a closed-workload user for its whole life) has a
//!   **behaviour stream** for what its usage behaviour draws (delays, loop counts, branch
//!   choices, the characterisations of entry-level system calls) and, closed, its think times,
//!   seeded from the scenario's seed and the user's number in its scenario (0, 1, ...);
//! * each user also has a **request stream** for everything else its process draws: the system
//!   work of its calls (SEFF actions, demands, failures). A forked behaviour gets a request
//!   stream of its own, seeded from a draw of its parent's.
//!
//! Draws outside processes (reconfiguration, scaling policies) stay on the run's stream. Two
//! runs with the same seed and the same usage model therefore see the same arrivals and every
//! user the same behaviour, whatever the system does; a user's request draws stay aligned as far
//! as the two systems execute the same actions for it, and a difference in one user's requests
//! does not shift any other user's numbers. This is common random numbers by dedicated streams
//! (Law, *Simulation Modeling and Analysis*, §11.2). Open users are numbered in arrival order,
//! the same in both runs; closed users in creation order (at the start, at load-profile steps and
//! population changes), also the same.
//!
//! Mechanics: a process's stream is swapped into `RunState::rng` while the process runs
//! (`run_process`), and a user's behaviour stream around each usage-level draw, so the evaluation
//! code is unchanged. Swaps nest (a process run synchronously inside another swaps its own stream
//! in and out). Fast mode only: the exact mode reproduces the reference's single stream.

use super::*;

/// SplitMix64's finaliser: a bijective mix of 64 bits.
#[inline]
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a over the bytes of `s` (a stable hash of a scenario id).
#[inline]
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The seed of scenario `id`'s streams in a run with seed `seed`.
pub(crate) fn scenario_seed(seed: i64, id: &str) -> u64 {
    mix(mix(seed as u64) ^ fnv1a(id))
}

/// The seed of the behaviour stream of user `k` (0-based, in creation order) of a scenario with
/// seed `scen`.
pub(crate) fn user_seed(scen: u64, k: u64) -> u64 {
    mix(scen ^ mix(k.wrapping_add(1)))
}

/// The seed of a user's request stream, from its behaviour stream's seed.
pub(crate) fn request_seed(user: u64) -> u64 {
    mix(user ^ 0x5245_5155_4553_5453)
}

/// The usage streams of a run (`RunState::us`).
#[derive(Clone, Debug)]
pub(crate) struct UsageStreams<R> {
    /// Per usage scenario (index of `CompiledModel::scenarios`): its seed, its arrival stream
    /// and the users created so far; grown when a model change adds scenarios.
    pub scen: Vec<Option<ScenStreams<R>>>,
    /// Per process slot: the streams of the process in it.
    pub slots: Vec<Option<SlotStreams<R>>>,
    /// The slots whose process stream is swapped in now, innermost last (empty between events).
    pub active: Vec<u32>,
}

impl<R> Default for UsageStreams<R> {
    fn default() -> Self {
        UsageStreams {
            scen: Vec::new(),
            slots: Vec::new(),
            active: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScenStreams<R> {
    pub seed: u64,
    pub arrivals: R,
    pub users: u64,
}

/// The streams of one process.
#[derive(Clone, Debug)]
pub(crate) struct SlotStreams<R> {
    /// A user's behaviour stream.
    pub behaviour: Option<R>,
    /// The process's own stream: a user's or a fork's request stream (`None` for a generator,
    /// which draws from its scenario's arrival stream).
    pub request: Option<R>,
    /// The scenario of an open workload's generator.
    pub generator: Option<u32>,
}

/// Whose stream a usage-level draw takes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Who {
    /// The behaviour stream of the user in a process slot.
    User(u32),
}

fn create<R: SimRng>(seed: u64) -> R {
    // fast mode only (checked in `Simulation::create`): own stream, no tape, cannot fail
    R::create(&RngMode::Own, seed as i64, false, false, false)
        .expect("a fast-mode stream is always created")
}

impl<R: SimRng> UsageStreams<R> {
    fn scen_mut(&mut self, cm: &CompiledModel, seed: i64, scen: u32) -> &mut ScenStreams<R> {
        let i = scen as usize;
        if self.scen.len() <= i {
            self.scen.resize_with(i + 1, || None);
        }
        self.scen[i].get_or_insert_with(|| {
            let id = &cm.scenario_ids[cm.scenarios[i].id.index()];
            let s = scenario_seed(seed, id);
            ScenStreams {
                seed: s,
                arrivals: create(s),
                users: 0,
            }
        })
    }

    fn set(&mut self, slot: u32, s: SlotStreams<R>) {
        let i = slot as usize;
        if self.slots.len() <= i {
            self.slots.resize_with(i + 1, || None);
        }
        self.slots[i] = Some(s);
    }

    /// A new user of scenario `scen` in process slot `slot`: its streams.
    pub fn new_user(&mut self, cm: &CompiledModel, seed: i64, scen: u32, slot: u32) {
        let sc = self.scen_mut(cm, seed, scen);
        let k = sc.users;
        sc.users += 1;
        let us = user_seed(sc.seed, k);
        self.set(
            slot,
            SlotStreams {
                behaviour: Some(create(us)),
                request: Some(create(request_seed(us))),
                generator: None,
            },
        );
    }

    /// The generator of scenario `scen` in process slot `slot`.
    pub fn new_generator(&mut self, cm: &CompiledModel, seed: i64, scen: u32, slot: u32) {
        self.scen_mut(cm, seed, scen);
        self.set(
            slot,
            SlotStreams {
                behaviour: None,
                request: None,
                generator: Some(scen),
            },
        );
    }

    /// A forked behaviour in process slot `slot`, its stream seeded with `seed`.
    pub fn new_fork(&mut self, slot: u32, seed: u64) {
        self.set(
            slot,
            SlotStreams {
                behaviour: None,
                request: Some(create(mix(seed))),
                generator: None,
            },
        );
    }

    /// The process in `slot` ended (outside its own run: its streams are not swapped in).
    pub fn end(&mut self, slot: u32) {
        if !self.active.contains(&slot)
            && let Some(u) = self.slots.get_mut(slot as usize)
        {
            *u = None;
        }
    }

    /// The process stream of `slot` (its generator's arrival stream or its request stream), if
    /// it has one.
    fn process_stream(&mut self, slot: u32) -> Option<&mut R> {
        let s = self.slots.get_mut(slot as usize)?.as_mut()?;
        match s.generator {
            Some(scen) => self
                .scen
                .get_mut(scen as usize)?
                .as_mut()
                .map(|x| &mut x.arrivals),
            None => s.request.as_mut(),
        }
    }
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// Process `slot` starts running: its stream goes in place of the run's (or of the stream of
    /// the process it runs inside).
    #[inline]
    pub(super) fn us_enter(&mut self, slot: u32) {
        if let Some(us) = self.st.us.as_deref_mut()
            && let Some(r) = us.process_stream(slot)
        {
            std::mem::swap(&mut self.st.rng, r);
            us.active.push(slot);
        }
    }

    /// Process `slot` stops running (it waits, ended or failed): the previous stream goes back;
    /// an ended process's streams are dropped.
    #[inline]
    pub(super) fn us_leave(&mut self, slot: u32) {
        let terminated = self.st.procs[slot as usize].state == PState::Terminated;
        if let Some(us) = self.st.us.as_deref_mut() {
            if us.active.last() == Some(&slot) {
                us.active.pop();
                if let Some(r) = us.process_stream(slot) {
                    std::mem::swap(&mut self.st.rng, r);
                }
            }
            if terminated {
                us.end(slot);
            }
        }
    }

    /// Process `slot` now holds a new user of scenario `scen`, the generator of `scen`, or a
    /// forked behaviour (seeded by a draw of the running process's stream).
    #[inline]
    pub(super) fn us_spawned(&mut self, slot: u32, kind: PKind, scen: Option<u32>) {
        if self.st.us.is_none() {
            return;
        }
        let (cm, seed) = (self.cm, self.st.cfg.seed);
        match (kind, scen) {
            (PKind::OpenUser | PKind::ClosedUser, Some(scen)) => {
                if let Some(us) = self.st.us.as_deref_mut() {
                    us.new_user(cm, seed, scen, slot);
                }
            }
            (PKind::OpenGen, Some(scen)) => {
                if let Some(us) = self.st.us.as_deref_mut() {
                    us.new_generator(cm, seed, scen, slot);
                }
            }
            (PKind::Forked { .. }, _) => {
                let u = self.st.rng.next_uniform();
                let s = (u * 9_007_199_254_740_992.0) as u64; // 53 bits
                if let Some(us) = self.st.us.as_deref_mut() {
                    us.new_fork(slot, s);
                }
            }
            _ => {}
        }
    }

    /// Process `slot` ended outside its own run.
    #[inline]
    pub(super) fn us_end(&mut self, slot: u32) {
        if let Some(us) = self.st.us.as_deref_mut() {
            us.end(slot);
        }
    }

    /// `f` with the current stream replaced by the behaviour stream of `who`.
    #[inline]
    pub(super) fn with_stream<T>(&mut self, who: Who, f: impl FnOnce(&mut Self) -> T) -> T {
        if self.st.us.is_none() {
            return f(self);
        }
        self.us_swap(who);
        let r = f(self);
        self.us_swap(who);
        r
    }

    /// Exchanges the current stream with the behaviour stream of `who` (no effect without usage
    /// streams or without such a stream). Called in pairs around a draw.
    #[inline]
    pub(super) fn us_swap(&mut self, who: Who) {
        let Who::User(slot) = who;
        if let Some(us) = self.st.us.as_deref_mut()
            && let Some(Some(s)) = us.slots.get_mut(slot as usize)
            && let Some(r) = s.behaviour.as_mut()
        {
            std::mem::swap(&mut self.st.rng, r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_distinct_and_stable() {
        let a = scenario_seed(1, "_us1");
        assert_eq!(a, scenario_seed(1, "_us1"));
        assert_ne!(a, scenario_seed(1, "_us2"));
        assert_ne!(a, scenario_seed(2, "_us1"));
        assert_ne!(user_seed(a, 0), user_seed(a, 1));
        assert_ne!(user_seed(a, 0), a);
        assert_ne!(request_seed(user_seed(a, 0)), user_seed(a, 0));
    }
}
