//! Usage streams (`SimConfig::usage_streams`, `docs/spec/random.md` RND-11): common random
//! numbers across models that share a usage model.
//!
//! With one stream per run (RND-1), two models that differ only in their system (another
//! component, another demand, another allocation) draw different numbers of uniforms between
//! two users, so the users of the two runs arrive at different times and take different paths
//! after the first few events. With usage streams, the usage model draws from streams of its own:
//!
//! * each usage scenario has an **arrival stream** for its open workload's inter-arrival times,
//!   seeded from the run seed and the scenario's id;
//! * each user (an open-workload user, or a closed-workload user for its whole life) has a
//!   **user stream** for everything its usage behaviour draws (delays, loop counts, branch
//!   choices, the characterisations of entry-level system calls) and, closed, its think times;
//!   it is seeded from the scenario's seed and the user's number in its scenario (0, 1, ...).
//!
//! The system's draws (SEFFs, resources, failures) stay on the run's stream. Two runs with the
//! same seed and the same usage model therefore see the same arrivals and every user the same
//! choices, whatever the system does: common random numbers in the sense of Law, *Simulation
//! Modeling and Analysis*, §11.2 (synchronisation by dedicated streams). Open users are numbered
//! in arrival order, which is the same in both runs; closed users in creation order (at the
//! start, at load-profile steps and population changes), also the same.
//!
//! Fast mode only: the exact mode reproduces the reference's single stream.

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

/// The seed of user `k` (0-based, in creation order) of a scenario with seed `scen`.
pub(crate) fn user_seed(scen: u64, k: u64) -> u64 {
    mix(scen ^ mix(k.wrapping_add(1)))
}

/// The usage streams of a run (`RunState::us`).
#[derive(Clone, Debug)]
pub(crate) struct UsageStreams<R> {
    /// Per usage scenario (index of `CompiledModel::scenarios`): its seed, its arrival stream
    /// and the users created so far; grown when a model change adds scenarios.
    pub scen: Vec<Option<ScenStreams<R>>>,
    /// Per process slot: the user stream of the user in it.
    pub user: Vec<Option<R>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ScenStreams<R> {
    pub seed: u64,
    pub arrivals: R,
    pub users: u64,
}

/// Whose stream a draw takes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Who {
    /// A scenario's arrival stream.
    Arrivals(u32),
    /// The user in a process slot.
    User(u32),
}

fn create<R: SimRng>(seed: u64) -> R {
    // fast mode only (checked in `Simulation::create`): own stream, no tape, cannot fail
    R::create(&RngMode::Own, seed as i64, false, false, false)
        .expect("a fast-mode stream is always created")
}

impl<R> Default for UsageStreams<R> {
    fn default() -> Self {
        UsageStreams {
            scen: Vec::new(),
            user: Vec::new(),
        }
    }
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

    /// A new user of scenario `scen` in process slot `slot`: its stream.
    pub fn new_user(&mut self, cm: &CompiledModel, seed: i64, scen: u32, slot: u32) {
        let sc = self.scen_mut(cm, seed, scen);
        let k = sc.users;
        sc.users += 1;
        let r = create(user_seed(sc.seed, k));
        let i = slot as usize;
        if self.user.len() <= i {
            self.user.resize_with(i + 1, || None);
        }
        self.user[i] = Some(r);
    }

    /// The process in `slot` ended.
    pub fn end(&mut self, slot: u32) {
        if let Some(u) = self.user.get_mut(slot as usize) {
            *u = None;
        }
    }

    /// The stream of `who` (created on first use for a scenario; a user without a stream, which
    /// cannot happen, gets one of seed 0 rather than a panic).
    pub fn get(&mut self, cm: &CompiledModel, seed: i64, who: Who) -> &mut R {
        match who {
            Who::Arrivals(scen) => &mut self.scen_mut(cm, seed, scen).arrivals,
            Who::User(slot) => {
                let i = slot as usize;
                if self.user.len() <= i {
                    self.user.resize_with(i + 1, || None);
                }
                self.user[i].get_or_insert_with(|| create(0))
            }
        }
    }
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// Exchanges the run's stream with the usage stream of `who` (no effect without usage
    /// streams). Called in pairs around a draw: the first call puts the usage stream in place,
    /// the second puts it back.
    #[inline]
    pub(super) fn us_swap(&mut self, who: Who) {
        if let Some(us) = self.st.us.as_deref_mut() {
            let r = us.get(self.cm, self.st.cfg.seed, who);
            std::mem::swap(&mut self.st.rng, r);
        }
    }

    /// Process `slot` now holds a new user of scenario `scen`.
    #[inline]
    pub(super) fn us_new_user(&mut self, scen: u32, slot: u32) {
        if let Some(us) = self.st.us.as_deref_mut() {
            us.new_user(self.cm, self.st.cfg.seed, scen, slot);
        }
    }

    /// Process `slot` ended.
    #[inline]
    pub(super) fn us_end(&mut self, slot: u32) {
        if let Some(us) = self.st.us.as_deref_mut() {
            us.end(slot);
        }
    }

    /// `f` with the run's stream replaced by the usage stream of `who`.
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
    }
}
