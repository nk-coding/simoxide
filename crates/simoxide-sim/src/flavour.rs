//! Simulator flavours: whose semantics a run follows (`DESIGN-SPD.md` §5.1-5.2).
//!
//! * [`SimuLizar`] (the default): SimuLizar 5.2.2, byte-exact with [`crate::Exact`]
//!   (`docs/spec/`).
//! * [`Slingshot`]: the Slingshot simulator (`docs/spec/slingshot/`, rules `SS-*`), in its *fast*
//!   mode: the semantics that change results, not Slingshot's bus events. Slingshot bugs are fixed
//!   by default (SimuLizar's semantics, decision U13); each documented quirk `Q1`-`Q37` has a
//!   compat flag ([`Quirk`], [`Quirks`]) that reproduces it (decision U5).
//!
//! A [`Simulation`](crate::Simulation) is generic over its flavour `F` like over its RNG policy
//! `C`: the hooks are associated constants and types, monomorphized, so the SimuLizar flavour
//! runs exactly the code it ran before flavours existed. The flavour of a run is the flavour its
//! [`CompiledModel`](crate::CompiledModel) was compiled for ([`FlavourConfig`],
//! [`CompiledModel::compile_flavour`](crate::CompiledModel::compile_flavour)); [`crate::run`]
//! dispatches on it.

use simoxide_sched::{Clock, F64Secs, Nanos};

/// A compile-time simulator flavour (implemented by [`SimuLizar`] and [`Slingshot`]).
pub trait Flavour: 'static {
    /// The run-time name of this flavour.
    const KIND: FlavourKind;
    /// Slingshot's semantics (the branches of the shared code that differ).
    const SLINGSHOT: bool;
    /// How points in time are encoded: SimuLizar's integer nanoseconds, Slingshot's `double`
    /// seconds (SS-ENG-2).
    type Clock: Clock;
}

/// SimuLizar 5.2.2 (the default flavour).
#[derive(Debug, Clone, Copy)]
pub struct SimuLizar;

impl Flavour for SimuLizar {
    const KIND: FlavourKind = FlavourKind::SimuLizar;
    const SLINGSHOT: bool = false;
    type Clock = Nanos;
}

/// Slingshot (nightly `1.0.0.202609010045`), fast mode.
#[derive(Debug, Clone, Copy)]
pub struct Slingshot;

impl Flavour for Slingshot {
    const KIND: FlavourKind = FlavourKind::Slingshot;
    const SLINGSHOT: bool = true;
    type Clock = F64Secs;
}

/// A flavour at run time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FlavourKind {
    #[default]
    SimuLizar,
    Slingshot,
    /// Slingshot's **exact mode** (`DESIGN-SPD.md` X2, `docs/spec/slingshot/exact.md`): Slingshot's
    /// bus cascade handler by handler, with every quirk (`Q1`-`Q37`) and the disposed handlers
    /// (Q18), byte-equal to the reference runner's `det` mode at level E. Run by
    /// [`crate::ssx`] (not by [`crate::Simulation`]); the model is compiled as for
    /// [`FlavourKind::Slingshot`] with [`Quirks::all`].
    SlingshotExact,
}

impl FlavourKind {
    /// `simulizar`, `slingshot`, `slingshot-exact`.
    pub fn name(self) -> &'static str {
        match self {
            FlavourKind::SimuLizar => "simulizar",
            FlavourKind::Slingshot => "slingshot",
            FlavourKind::SlingshotExact => "slingshot-exact",
        }
    }

    /// Parses [`FlavourKind::name`].
    pub fn parse(s: &str) -> Option<FlavourKind> {
        match s {
            "simulizar" => Some(FlavourKind::SimuLizar),
            "slingshot" => Some(FlavourKind::Slingshot),
            "slingshot-exact" => Some(FlavourKind::SlingshotExact),
            _ => None,
        }
    }

    /// Slingshot's semantics (fast or exact mode).
    #[inline]
    pub fn is_slingshot(self) -> bool {
        matches!(self, FlavourKind::Slingshot | FlavourKind::SlingshotExact)
    }
}

impl std::fmt::Display for FlavourKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The flavour a model is compiled for, with the Slingshot compat flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FlavourConfig {
    pub kind: FlavourKind,
    /// Slingshot quirks to reproduce (ignored by the SimuLizar flavour; must be empty there).
    pub quirks: Quirks,
}

impl FlavourConfig {
    /// SimuLizar.
    pub const SIMULIZAR: FlavourConfig = FlavourConfig {
        kind: FlavourKind::SimuLizar,
        quirks: Quirks::NONE,
    };

    /// Slingshot with the fixed behaviour of every quirk.
    pub const SLINGSHOT: FlavourConfig = FlavourConfig {
        kind: FlavourKind::Slingshot,
        quirks: Quirks::NONE,
    };

    /// Slingshot's exact mode: every quirk ([`FlavourKind::SlingshotExact`]).
    pub fn slingshot_exact() -> FlavourConfig {
        FlavourConfig {
            kind: FlavourKind::SlingshotExact,
            quirks: Quirks::all(),
        }
    }

    /// Slingshot reproducing `quirks`.
    pub fn slingshot(quirks: Quirks) -> FlavourConfig {
        FlavourConfig {
            kind: FlavourKind::Slingshot,
            quirks,
        }
    }

    /// Is the Slingshot flavour reproducing quirk `q`?
    #[inline]
    pub fn has(&self, q: Quirk) -> bool {
        self.quirks.has(q)
    }
}

/// A documented Slingshot quirk (`docs/spec/slingshot/quirks.md`, `DESIGN-SPD.md` §1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Quirk(u8);

/// How far the fast mode reproduces a quirk under its flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// Reproduced.
    Full,
    /// Reproduced up to Slingshot's exception, which is a run error here: what Slingshot does
    /// after the throw (the disposed handler, `Q18`) is the exact mode's (phase X2).
    UpToThrow,
    /// A quirk of the SPD interpreter or adjustment: read by `simoxide-spd` (phases P1-P3).
    Spd,
    /// Needs the Slingshot-exact mode's handler steps (phase X2): refused in the fast mode.
    ExactOnly,
}

macro_rules! quirks {
    ($($q:ident = $n:literal, $support:ident, $summary:literal;)*) => {
        impl Quirk {
            $(
                #[doc = $summary]
                pub const $q: Quirk = Quirk($n);
            )*
            /// Every quirk, in number order.
            pub const ALL: &'static [Quirk] = &[$(Quirk::$q),*];

            /// One line: what the flag reproduces.
            pub fn summary(self) -> &'static str {
                match self.0 {
                    $($n => $summary,)*
                    _ => "",
                }
            }

            /// How far the fast mode reproduces it.
            pub fn support(self) -> Support {
                match self.0 {
                    $($n => Support::$support,)*
                    _ => Support::Full,
                }
            }
        }
    };
}

quirks! {
    Q1 = 1, Spd, "doOnDisregard does not restore the adjustor state";
    Q2 = 2, Spd, "the constraint state changes when the request is made, even if QVTo fails";
    Q3 = 3, Spd, "AVERAGE is an EMA (60 s, 10 s, 0.2), the other aggregations use the last 10 values (kept: the fixed behaviour equals it)";
    Q4 = 4, Spd, "a relative adjustment of 0 % ignores minAdjustmentValue";
    Q5 = 5, Spd, "a scale-in aborts the one job with a pending event on a removed container and loses the others";
    Q6 = 6, Spd, "after an adjustment every linking resource connects every container";
    Q7 = 7, Spd, "top-down scale-in removes emptied containers one enactment late";
    Q8 = 8, Spd, "top-down placement uses ->any(true) on a set (Java-hash order)";
    Q9 = 9, Spd, "only thrashing constraints are checked in the chain";
    Q10 = 10, Spd, "TargetGroupUtils keeps state in static fields (not reproduced: no shared JVM state)";
    Q11 = 11, Spd, "unsupported stimuli and SimpleFireOnTrend: NullPointerException at set-up";
    Q12 = 12, Spd, "composed triggers AND/OR/XOR misbehave";
    Q13 = 13, Spd, "instantaneous samples merged over containers; getResult on unaggregated events";
    Q14 = 14, Spd, "deleted elements not reported, monitors of removed containers stay, population changes ineffective";
    Q15 = 15, Spd, "policies react to each other's SimulationTimeReached";
    Q16 = 16, Spd, "container order after an adjustment follows Java identity hashes";
    Q17 = 17, ExactOnly, "the demands of one InternalAction run concurrently and each completion advances the cursor";
    Q18 = 18, ExactOnly, "a throwing handler is disposed and the run goes on";
    Q19 = 19, UpToThrow, "loop count 0: a SEFF loop runs its body once, then throws; a usage loop throws";
    Q20 = 20, Full, "roles resolved by role id alone (entry calls: first assembly of the component; external calls: first system connector with the required role)";
    Q21 = 21, UpToThrow, "a guarded branch without a true guard throws";
    Q22 = 22, UpToThrow, "CollectionIteratorAction: INNER drawn once, the frame is never popped, the end of the request throws";
    Q23 = 23, UpToThrow, "ForkAction: asynchronous behaviours ignored, synchronous ones share the user's stack and cursor (more than one throws)";
    Q24 = 24, UpToThrow, "Acquire/Release without a ParametricResourceDemand throw";
    Q25 = 25, Full, "no delay resources: a demand on a Delay (or missing) resource aborts the user";
    Q26 = 26, Full, "stale processor-sharing completions record state and utilisation samples";
    Q27 = 27, Full, "passive capacities are evaluated without a frame";
    Q28 = 28, Full, "every link transfer draws Math.random() for its failure check, also at probability 0; a failed transfer stalls the link";
    Q29 = 29, Full, "response-time monitors of entry-level calls, system operations and external calls are ignored";
    Q30 = 30, UpToThrow, "composite components: entry into one stops the user silently; entered through a connector, its demands abort";
    Q31 = 31, UpToThrow, "an infrastructure call breaks the assembly-operation monitor";
    Q32 = 32, Full, "simTime is the only stop condition (the maximum measurement count is ignored)";
    Q33 = 33, Full, "the assembly-operation key of an entry-level call uses the system's role";
    Q34 = 34, Full, "probes per resource specification (replicaID ignored) and per passive resource (assembly contexts merged)";
    Q35 = 35, Full, "unsupported actions, ResourceCalls, missing connectors or SEFFs end a request silently";
    Q36 = 36, Full, "infrastructure-call inputs are ignored";
    Q37 = 37, UpToThrow, "calls between containers without a connecting link: a link touching either container is used, none throws";
}

impl Quirk {
    /// The quirk number (`Qn`).
    pub fn number(self) -> u8 {
        self.0
    }

    /// `Q1`..`Q37`.
    pub fn parse(s: &str) -> Option<Quirk> {
        let n: u8 = s.strip_prefix(['Q', 'q'])?.parse().ok()?;
        Quirk::ALL.iter().copied().find(|q| q.0 == n)
    }
}

impl std::fmt::Display for Quirk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Q{}", self.0)
    }
}

/// A set of [`Quirk`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Quirks(u64);

impl Quirks {
    /// No quirk: the fixed behaviour everywhere (the default).
    pub const NONE: Quirks = Quirks(0);

    /// Every quirk (what the Slingshot-exact mode runs with).
    pub fn all() -> Quirks {
        Quirk::ALL.iter().fold(Quirks::NONE, |s, &q| s.with(q))
    }

    /// The quirks the fast mode can reproduce or reproduce up to the throw (everything but
    /// [`Support::ExactOnly`]).
    pub fn all_fast() -> Quirks {
        Quirk::ALL
            .iter()
            .filter(|q| q.support() != Support::ExactOnly)
            .fold(Quirks::NONE, |s, &q| s.with(q))
    }

    #[inline]
    pub const fn has(self, q: Quirk) -> bool {
        self.0 & (1 << q.0) != 0
    }

    #[must_use]
    pub const fn with(self, q: Quirk) -> Quirks {
        Quirks(self.0 | (1 << q.0))
    }

    #[must_use]
    pub const fn without(self, q: Quirk) -> Quirks {
        Quirks(self.0 & !(1 << q.0))
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The quirks in the set, in number order.
    pub fn iter(self) -> impl Iterator<Item = Quirk> {
        Quirk::ALL.iter().copied().filter(move |&q| self.has(q))
    }

    /// Parses a comma-separated list: `Q26,Q29`, `all`, `fast` ([`Quirks::all_fast`]), `none`,
    /// and `-Qn` to remove one (`fast,-Q32`).
    pub fn parse(s: &str) -> Result<Quirks, String> {
        let mut q = Quirks::NONE;
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part {
                "all" => q = Quirks::all(),
                "fast" => q = Quirks::all_fast(),
                "none" => q = Quirks::NONE,
                p => {
                    let (neg, name) = match p.strip_prefix('-') {
                        Some(n) => (true, n),
                        None => (false, p),
                    };
                    let k = Quirk::parse(name).ok_or_else(|| {
                        format!("unknown quirk '{name}' (Q1..Q37, all, fast, none)")
                    })?;
                    q = if neg { q.without(k) } else { q.with(k) };
                }
            }
        }
        Ok(q)
    }
}

impl std::fmt::Display for Quirks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v: Vec<String> = self.iter().map(|q| q.to_string()).collect();
        if v.is_empty() {
            f.write_str("none")
        } else {
            f.write_str(&v.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quirk_sets_parse_and_print() {
        let q = Quirks::parse("Q26, q29").unwrap();
        assert!(q.has(Quirk::Q26) && q.has(Quirk::Q29) && !q.has(Quirk::Q1));
        assert_eq!(q.to_string(), "Q26,Q29");
        assert_eq!(Quirks::parse("all").unwrap().iter().count(), 37);
        let f = Quirks::parse("fast,-Q32").unwrap();
        assert!(
            !f.has(Quirk::Q17) && !f.has(Quirk::Q18) && !f.has(Quirk::Q32) && f.has(Quirk::Q26)
        );
        assert!(Quirks::parse("Q38").is_err());
        assert_eq!(Quirks::NONE.to_string(), "none");
        assert_eq!(Quirk::ALL.len(), 37);
        assert!(Quirk::ALL.iter().all(|q| !q.summary().is_empty()));
    }
}
