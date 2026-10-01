//! Random PCM model generator (the fuzzer's input).
//!
//! Generates structurally valid models covering the v1 feature set: composite components, call chains
//! (a DAG: no recursion), probabilistic and guarded branches, loops, collection iterators, sync/async
//! forks, passive resources, set variables (return values), component and assembly parameters,
//! infrastructure calls, resource calls, PS/FCFS/delay (multi-core) resources, linking resources,
//! open/closed workloads, several scenarios, StoEx distributions and parametric dependencies.
//!
//! Seeded and reproducible: [`generate`] is a pure function of [`GenConfig`]. `size` (1..=10) scales
//! the model; [`Features`] switches individual features off (used by the fuzzer's minimizer).
//! The generator estimates the expected load of every resource and sets arrival rates / think times
//! so that utilizations stay below ~0.7, which keeps reference runs short.
//!
//! Reference limitations avoided on purpose (they crash SimuLizar 5.2.2, `docs/correctness/testing.md`):
//! recursion (response-time monitors fail), guarded branches without a true guard, set-variable
//! actions in forked behaviours or infrastructure SEFFs, missing links between containers.

mod build;
pub mod model;
pub mod xmi;

pub use model::GenModel;

/// Feature switches (probabilities / weights in [0, 1]; 0 disables a feature).
#[derive(Clone, Debug, PartialEq)]
pub struct Features {
    pub multi_container: f64,
    pub hdd: f64,
    pub delay_resource: f64,
    pub multicore: f64,
    pub fcfs_cpu: f64,
    pub stoex_rate: f64,
    pub composite: f64,
    pub double_assembly: f64,
    pub component_params: f64,
    pub infra: f64,
    pub resource_call: f64,
    pub passive: f64,
    pub external_call: f64,
    pub return_value: f64,
    pub prob_branch: f64,
    pub guarded_branch: f64,
    pub loops: f64,
    pub collection: f64,
    pub fork_sync: f64,
    pub fork_async: f64,
    pub fork_calls: f64,
    pub distributions: f64,
    pub parametric: f64,
    pub bytesize: f64,
    pub usage_delay: f64,
    pub usage_branch: f64,
    pub usage_loop: f64,
    pub closed: f64,
    pub multi_scenario: f64,
    pub no_link_throughput: f64,
    /// Utilisation target 0.75-0.92 instead of 0.25-0.65 (long queues, many in-flight requests at
    /// the stop, heavy post-stop drain).
    pub heavy_load: f64,
    /// Equal constant demands, think and inter-arrival times (simultaneous events, zero demands,
    /// events at exactly the max simulation time).
    pub ties: f64,
    /// Nesting depth up to 5 (instead of 3), forks inside forks, larger SEFFs.
    pub deep_nesting: f64,
    /// Unusual StoEx: PMF literals that need normalisation or sorting, `Pois` (which can return
    /// -1), integer overflow and division, `?:`, `^`, `BoolPMF` guards, sub-nanosecond demands,
    /// branch probabilities that do not sum to 1 or are 0, empty collections.
    pub stoex_exotic: f64,
    /// Sync forks whose children finish without waiting (the parent gets several Resume notes;
    /// the reference sometimes crashes, those models are skipped).
    pub double_resume: f64,
    /// HDDProcessingResourceSpecification with read/write rates and HDD read/write resource calls.
    pub hdd_rw: f64,
    /// Component parameters with distributions and BYTESIZE, overrides on every assembly context
    /// and on composite inner assemblies.
    pub param_override: f64,
    /// Sliding-window utilisation monitors (TimeDriven) and TimeDrivenAggregation on scenarios.
    pub windows: f64,
    /// Long runs: 300-3000 measurements (off by default; campaigns set it explicitly).
    pub long_run: f64,
    /// The back component of a composite is itself a composite (two levels of provided delegation).
    pub nested_composite: f64,
    /// Response-time monitors on assembly operations (AssemblyOperationMeasuringPoint), system
    /// level and inside composites.
    pub extra_monitors: f64,
    /// Measurement specifications with `triggersSelfAdaptations = true`: all of them (attribute
    /// omitted, the EMF default) or a random subset. The reference then creates its
    /// reconfiguration process at the first runtime-measurement write (MEAS-7.2).
    pub triggers: f64,
    /// With `triggers`: a FixedSizeAggregation or VariableSizeAggregation (PRM only) replaces
    /// the FeedThrough of a scenario's response time.
    pub prm_aggregation: f64,
    /// A nested resource container (with a CPU) inside a container. SimuLizar 5.2.2 simulates
    /// top-level containers only: the nested one is ignored, and a component allocated to it
    /// (a quarter of these models) aborts the run at its first resource demand.
    pub nested_container: f64,
    /// `simulate_linking_resources = true` (middleware marshalling): every external call passes
    /// `stream.BYTESIZE` and every operation SEFF sets it for the reply. In a fifth of these
    /// models the stream is missing and the reference aborts at the first assembly-connector call.
    pub middleware_stream: f64,
}

impl Default for Features {
    fn default() -> Self {
        Features {
            multi_container: 0.5,
            hdd: 0.3,
            delay_resource: 0.2,
            multicore: 0.25,
            fcfs_cpu: 0.3,
            stoex_rate: 0.1,
            composite: 0.3,
            double_assembly: 0.2,
            component_params: 0.3,
            infra: 0.25,
            resource_call: 0.15,
            passive: 0.3,
            external_call: 1.0,
            return_value: 0.3,
            prob_branch: 0.6,
            guarded_branch: 0.6,
            loops: 0.6,
            collection: 0.5,
            fork_sync: 0.4,
            fork_async: 0.3,
            fork_calls: 0.3,
            distributions: 1.0,
            parametric: 1.0,
            bytesize: 0.4,
            usage_delay: 0.3,
            usage_branch: 0.3,
            usage_loop: 0.3,
            closed: 0.5,
            multi_scenario: 0.3,
            no_link_throughput: 0.15,
            heavy_load: 0.15,
            ties: 0.15,
            deep_nesting: 0.15,
            stoex_exotic: 0.25,
            double_resume: 0.1,
            hdd_rw: 0.5,
            param_override: 0.3,
            windows: 0.15,
            long_run: 0.0,
            nested_composite: 0.4,
            extra_monitors: 0.2,
            triggers: 0.3,
            prm_aggregation: 0.3,
            nested_container: 0.1,
            middleware_stream: 0.1,
        }
    }
}

macro_rules! feature_list {
    ($($f:ident),* $(,)?) => {
        /// Names of all features (for `--disable` and the minimizer).
        pub const FEATURE_NAMES: &[&str] = &[$(stringify!($f)),*];
        impl Features {
            /// Sets a feature by name; false if unknown.
            pub fn set(&mut self, name: &str, v: f64) -> bool {
                match name {
                    $(stringify!($f) => { self.$f = v; true })*
                    _ => false,
                }
            }
            pub fn get(&self, name: &str) -> Option<f64> {
                match name {
                    $(stringify!($f) => Some(self.$f),)*
                    _ => None,
                }
            }
            /// All features off (a single component, single container, open workload).
            pub fn none() -> Self {
                Features { $($f: 0.0),* }
            }
        }
    };
}

feature_list!(
    multi_container,
    hdd,
    delay_resource,
    multicore,
    fcfs_cpu,
    stoex_rate,
    composite,
    double_assembly,
    component_params,
    infra,
    resource_call,
    passive,
    external_call,
    return_value,
    prob_branch,
    guarded_branch,
    loops,
    collection,
    fork_sync,
    fork_async,
    fork_calls,
    distributions,
    parametric,
    bytesize,
    usage_delay,
    usage_branch,
    usage_loop,
    closed,
    multi_scenario,
    no_link_throughput,
    heavy_load,
    ties,
    deep_nesting,
    stoex_exotic,
    double_resume,
    hdd_rw,
    param_override,
    windows,
    long_run,
    nested_composite,
    extra_monitors,
    triggers,
    prm_aggregation,
    nested_container,
    middleware_stream,
);

impl Features {
    /// The features of the original generator (all switches added later are off): reproduces the
    /// models of earlier campaigns seed for seed.
    pub fn classic() -> Self {
        let mut f = Features::default();
        for n in [
            "heavy_load",
            "ties",
            "deep_nesting",
            "stoex_exotic",
            "double_resume",
            "hdd_rw",
            "param_override",
            "windows",
            "long_run",
            "nested_composite",
            "extra_monitors",
            "triggers",
            "prm_aggregation",
            "nested_container",
            "middleware_stream",
        ] {
            f.set(n, 0.0);
        }
        f
    }
}

/// Generator input.
#[derive(Clone, Debug, PartialEq)]
pub struct GenConfig {
    /// Model name: directory name, file base name and id prefix.
    pub name: String,
    pub seed: u64,
    /// 1..=10: number of components, containers, actions per SEFF, scenarios.
    pub size: u32,
    pub features: Features,
}

impl GenConfig {
    pub fn new(name: impl Into<String>, seed: u64, size: u32) -> Self {
        GenConfig {
            name: name.into(),
            seed,
            size: size.clamp(1, 10),
            features: Features::default(),
        }
    }
}

/// Generates a model (pure function of the config).
pub fn generate(cfg: &GenConfig) -> GenModel {
    build::Builder::new(cfg).build()
}

/// SplitMix64: small, fast, reproducible across platforms and crate versions.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    /// Uniform in lo..=hi.
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }
    pub fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && self.f64() < p
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
    /// Index by weights (all zero -> None).
    pub fn weighted(&mut self, w: &[f64]) -> Option<usize> {
        let total: f64 = w.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = self.f64() * total;
        for (i, &wi) in w.iter().enumerate() {
            if x < wi {
                return Some(i);
            }
            x -= wi;
        }
        w.iter().rposition(|&wi| wi > 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let c = GenConfig::new("g", 42, 5);
        let a = generate(&c);
        let b = generate(&c);
        let d = std::env::temp_dir().join(format!("testkit-gen-det-{}", std::process::id()));
        let (da, db) = (d.join("a"), d.join("b"));
        xmi::write_model(&a, &da).unwrap();
        xmi::write_model(&b, &db).unwrap();
        for f in std::fs::read_dir(&da).unwrap() {
            let f = f.unwrap().path();
            let g = db.join(f.file_name().unwrap());
            assert_eq!(std::fs::read(&f).unwrap(), std::fs::read(&g).unwrap());
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn feature_names() {
        let mut f = Features::default();
        for n in FEATURE_NAMES {
            assert!(f.set(n, 0.0), "{n}");
        }
        assert_eq!(f, Features::none());
    }
}
