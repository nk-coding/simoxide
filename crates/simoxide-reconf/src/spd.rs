//! Slingshot's SPD adjustment (`DESIGN-SPD.md` §1.3, §5.4, P2): `MainTransformation.qvto` on
//! the run's model, as `SpdAdjustmentBehavior` runs it.
//!
//! - **Transformations.** Slingshot's transformation bundle (17 units, Palladio-Addons-SPD-Metamodel
//!   at a pinned commit) is embedded in the binary: the licence-headed copies of
//!   `crates/qvto/qvto-testkit/corpus/spd/transformations` ("licence pending confirmation",
//!   `DESIGN-SPD.md` U3: nothing is published until confirmed). Three units have patched copies
//!   for the provisional fixed behaviours of the quirks Q4, Q6 and Q7
//!   (`transformations/spd-fixed`); [`SpdVariant`] picks the unchanged unit when the quirk's
//!   compat flag is on, so [`simoxide_spd::Quirks::slingshot`] runs Slingshot's bundle as it is.
//!   [`main_transformation`] compiles a variant once per core ([`QvtoCore::compiled`]).
//! - **Running it.** [`QvtoAdjuster::spd`]: Slingshot's bindings (every model by namespace URI:
//!   allocation, system, resource environment, repository, semantic configuration and usage
//!   model `inout`, SPD `in`), `keepModeling = true`, QVTo's ordered mode by default (the `det`
//!   oracle's insertion-ordered collections; fast mode as an option, never compat), seeded ids
//!   for every PCM object the transformation creates (Slingshot's generated classes give each a
//!   UUID; `Wiring` compares ids), and [`SlingshotHooks`]:
//!   - before the run: `Configuration.enactedPolicy` := the request's policy, the container and
//!     allocation lists ([`simoxide_spd::adjusted::Before`]) and the model summary;
//!   - after it: Slingshot's `ModelAdjusted` ([`ModelAdjusted`]: old, new and deleted
//!     containers, new allocation contexts, the monitors Slingshot clones), the summary after,
//!     the target-group membership read from the changed model, the group sizes.
//!
//!   The journal goes through the change path ([`crate::QvtoRun::invoke`]); a change the
//!   simulator refuses ends the SPD run with its error.

use crate::adjust::{AdjustHooks, QvtoAdjuster};
use crate::host::{Outcome, QvtMode, QvtoCore, QvtoRun, ReconfError, RunConfig, Transformation};
use qvto_emf::IdPolicy;
use qvto_model::{Datum, ModelStore};
use simoxide_model::meta::feat as mf;
use simoxide_model::raw::Graph;
use simoxide_spd::adjusted::{Before, ModelAdjusted, input_ids, summary};
use simoxide_spd::sim::AdjustOutcome;
use simoxide_spd::{AdjustmentRequest, Membership, Quirks, Spd, SpdState};
use std::collections::HashSet;
use std::rc::Rc;

macro_rules! unit {
    ($qn:literal, $file:literal, $path:literal) => {
        (
            $qn,
            $file,
            include_str!(concat!(
                "../../qvto/qvto-testkit/corpus/spd/transformations/",
                $path
            )),
        )
    };
}

/// Slingshot's SPD transformation bundle: (qualified name, file name, text).
pub const TRANSFORMATIONS: [(&str, &str, &str); 17] = [
    unit!(
        "spd.MainTransformation",
        "MainTransformation.qvto",
        "spd/MainTransformation.qvto"
    ),
    unit!(
        "pcm.helpers.Commons",
        "Commons.qvto",
        "pcm/helpers/Commons.qvto"
    ),
    unit!(
        "pcm.helpers.Constructors",
        "Constructors.qvto",
        "pcm/helpers/Constructors.qvto"
    ),
    unit!(
        "pcm.helpers.Loadbalancing",
        "Loadbalancing.qvto",
        "pcm/helpers/Loadbalancing.qvto"
    ),
    unit!(
        "pcm.helpers.Wiring",
        "Wiring.qvto",
        "pcm/helpers/Wiring.qvto"
    ),
    unit!(
        "spd.bottomup.BottomUpTransformation",
        "BottomUpTransformation.qvto",
        "spd/bottomup/BottomUpTransformation.qvto"
    ),
    unit!(
        "spd.bottomup.BottomUpTransformationInfrastructure",
        "BottomUpTransformationInfrastructure.qvto",
        "spd/bottomup/BottomUpTransformationInfrastructure.qvto"
    ),
    unit!(
        "spd.bottomup.BottomUpTransformationServices",
        "BottomUpTransformationServices.qvto",
        "spd/bottomup/BottomUpTransformationServices.qvto"
    ),
    unit!(
        "spd.topdown.TopDownTransformation",
        "TopDownTransformation.qvto",
        "spd/topdown/TopDownTransformation.qvto"
    ),
    unit!(
        "spd.topdown.TopDownTransformationInfrastructure",
        "TopDownTransformationInfrastructure.qvto",
        "spd/topdown/TopDownTransformationInfrastructure.qvto"
    ),
    unit!(
        "spd.topdown.TopDownTransformationServices",
        "TopDownTransformationServices.qvto",
        "spd/topdown/TopDownTransformationServices.qvto"
    ),
    unit!(
        "spd.common.AdjustmentCalculator",
        "AdjustmentCalculator.qvto",
        "spd/common/AdjustmentCalculator.qvto"
    ),
    unit!(
        "spd.common.CompetingConsumersGroupPCMTransformation",
        "CompetingConsumersGroupPCMTransformation.qvto",
        "spd/common/CompetingConsumersGroupPCMTransformation.qvto"
    ),
    unit!(
        "spd.common.ResourceEnvironmentTransformation",
        "ResourceEnvironmentTransformation.qvto",
        "spd/common/ResourceEnvironmentTransformation.qvto"
    ),
    unit!(
        "spd.common.ServiceGroupRepositoryTransformation",
        "ServiceGroupRepositoryTransformation.qvto",
        "spd/common/ServiceGroupRepositoryTransformation.qvto"
    ),
    unit!(
        "spd.common.queries.SystemQueries",
        "SystemQueries.qvto",
        "spd/common/queries/SystemQueries.qvto"
    ),
    unit!(
        "spd.common.queries.TargetGroupQueries",
        "TargetGroupQueries.qvto",
        "spd/common/queries/TargetGroupQueries.qvto"
    ),
];

/// The patched units of the provisional fixed behaviours (`transformations/spd-fixed`, marked
/// "PATCHED by SimOxide"): (quirk, qualified name, text).
pub const FIXED: [(&str, &str, &str); 3] = [
    (
        "Q4",
        "spd.common.AdjustmentCalculator",
        include_str!("../transformations/spd-fixed/AdjustmentCalculator.qvto"),
    ),
    (
        "Q6",
        "spd.common.ResourceEnvironmentTransformation",
        include_str!("../transformations/spd-fixed/ResourceEnvironmentTransformation.qvto"),
    ),
    (
        "Q7",
        "spd.topdown.TopDownTransformationInfrastructure",
        include_str!("../transformations/spd-fixed/TopDownTransformationInfrastructure.qvto"),
    ),
];

/// Which units of the bundle run: Slingshot's (`true`, the quirk's compat flag is on) or the
/// patched copy of the fixed behaviour (`false`, the default).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpdVariant {
    /// Q4: `AdjustmentCalculator` (relative adjustment of 0 %).
    pub q4: bool,
    /// Q6: `ResourceEnvironmentTransformation` (every link connects every container).
    pub q6: bool,
    /// Q7: `TopDownTransformationInfrastructure` (emptied containers removed one enactment
    /// late).
    pub q7: bool,
}

impl SpdVariant {
    /// The variant of the interpreter's compat flags.
    pub fn of(q: &Quirks) -> SpdVariant {
        SpdVariant {
            q4: q.q4_relative_zero,
            q6: q.q6_all_links,
            q7: q.q7_late_removal,
        }
    }

    /// Slingshot's bundle unchanged.
    pub fn slingshot() -> SpdVariant {
        SpdVariant {
            q4: true,
            q6: true,
            q7: true,
        }
    }

    /// Every patched unit (the default).
    pub fn fixed() -> SpdVariant {
        SpdVariant::default()
    }

    fn key(self) -> String {
        format!(
            "simoxide-spd:MainTransformation:{}{}{}",
            u8::from(self.q4),
            u8::from(self.q6),
            u8::from(self.q7)
        )
    }

    /// The text of the unit `qualified_name` in this variant.
    pub fn text(self, qualified_name: &str) -> Option<&'static str> {
        let patched = FIXED.iter().find(|(q, qn, _)| {
            *qn == qualified_name
                && match *q {
                    "Q4" => !self.q4,
                    "Q6" => !self.q6,
                    _ => !self.q7,
                }
        });
        match patched {
            Some((_, _, text)) => Some(text),
            None => TRANSFORMATIONS
                .iter()
                .find(|(qn, ..)| *qn == qualified_name)
                .map(|(_, _, t)| *t),
        }
    }
}

/// Resolves the embedded units of a variant.
struct Embedded(SpdVariant);

impl Embedded {
    fn unit(&self, qualified_name: &str) -> Option<qvto::UnitSource> {
        let (qn, file, _) = TRANSFORMATIONS
            .iter()
            .find(|(qn, ..)| *qn == qualified_name)?;
        let text = self.0.text(qn)?;
        Some(qvto::UnitSource {
            key: format!("{}/{}", self.0.key(), qn),
            qualified_name: qn.to_string(),
            file_name: file.to_string(),
            path: None,
            text: Rc::from(text),
            blackbox: None,
        })
    }
}

impl qvto::UnitResolver for Embedded {
    fn resolve(&self, _: &qvto::UnitSource, qualified_name: &str) -> Option<qvto::UnitSource> {
        self.unit(qualified_name)
    }
}

/// Slingshot's `MainTransformation` in `variant`, compiled once per core.
pub fn main_transformation(
    core: &QvtoCore,
    variant: SpdVariant,
) -> Result<Transformation, ReconfError> {
    core.compiled(&variant.key(), |core| {
        let res = Embedded(variant);
        let main = res
            .unit("spd.MainTransformation")
            .expect("the embedded bundle has MainTransformation");
        core.compile(main, &res)
    })
}

/// How [`QvtoAdjuster::spd`] runs the adjustment.
#[derive(Clone, Debug)]
pub struct SpdAdjust {
    /// QVTo's mode: the ordered mode by default (the Slingshot reference's `det` collection
    /// order, `DESIGN-SPD.md` Q2), or the fast mode. Compat mode is never needed.
    pub mode: QvtMode,
    pub variant: SpdVariant,
    /// Ids of the PCM objects the transformation creates (seeded UUIDs by default).
    pub ids: IdPolicy,
    /// Record the model summaries before and after (the decision log's `before`/`after`).
    pub summaries: bool,
}

impl SpdAdjust {
    /// The defaults for an interpreter with the compat flags `quirks`: ordered mode, the
    /// variant of the flags, seeded ids, summaries.
    pub fn new(quirks: &Quirks) -> SpdAdjust {
        SpdAdjust {
            mode: QvtMode::InsertionOrderSets,
            variant: SpdVariant::of(quirks),
            ids: RunConfig::default().ids,
            summaries: true,
        }
    }
}

/// The [`AdjustHooks`] of Slingshot's adjustment (see the module documentation).
#[derive(Default)]
pub struct SlingshotHooks {
    summaries: bool,
    input: Option<HashSet<Box<str>>>,
    before: Before,
    summary_before: Option<Box<str>>,
}

impl SlingshotHooks {
    pub fn new(summaries: bool) -> SlingshotHooks {
        SlingshotHooks {
            summaries,
            ..SlingshotHooks::default()
        }
    }
}

impl AdjustHooks for SlingshotHooks {
    fn prepare(&mut self, base: &Graph) {
        if self.summaries && self.input.is_none() {
            self.input = Some(input_ids(base));
        }
    }

    fn before(&mut self, store: &mut Graph, spd: &Spd, req: &AdjustmentRequest) {
        let model = spd.model();
        if let Some(cfg) = model.configuration {
            let policy = model.policies[req.policy].obj;
            // `SpdAdjustmentBehavior`: setEnactedPolicy before the copies of the lists
            store
                .set(cfg, mf::Configuration_enactedPolicy, Datum::Obj(policy))
                .expect("Configuration.enactedPolicy");
        }
        self.before = Before::capture(store, model);
        self.summary_before = self
            .input
            .as_ref()
            .map(|input| summary(store, input).into());
    }

    fn after(
        &mut self,
        graph: &Graph,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
        outcome: Result<&Outcome, &ReconfError>,
    ) -> Result<AdjustOutcome, String> {
        let o = outcome.map_err(|e| e.to_string())?;
        let group = &spd.model().groups[req.group];
        let size_before = state.membership.group_size(group);
        if o.change.is_some() || o.succeeded {
            state.membership = Membership::from_graph(graph, spd.model());
        }
        let adjusted = if o.succeeded {
            ModelAdjusted::compute(&self.before, graph, spd.model())
        } else {
            ModelAdjusted::failed()
        };
        Ok(AdjustOutcome {
            ok: o.succeeded,
            size_before,
            size_after: state.membership.group_size(group),
            changes: Some(adjusted.changes()),
            before: self.summary_before.take(),
            after: self
                .input
                .as_ref()
                .map(|input| summary(graph, input).into()),
            adjusted: Some(adjusted),
        })
    }
}

impl QvtoAdjuster {
    /// Slingshot's SPD adjustment (see the module documentation): `MainTransformation` of
    /// `opts.variant`, compiled once per core, on a fresh QVTo state of the run.
    pub fn spd(core: &QvtoCore, opts: &SpdAdjust) -> Result<QvtoAdjuster, ReconfError> {
        let t = main_transformation(core, opts.variant)?;
        let run = QvtoRun::new(
            core,
            RunConfig {
                mode: opts.mode,
                ids: opts.ids,
                ..RunConfig::default()
            },
        )?;
        Ok(QvtoAdjuster::new(run, t).with_hooks(SlingshotHooks::new(opts.summaries)))
    }

    /// The adjuster of a fork of an SPD run ([`QvtoAdjuster::spd`]): a copy of the run's QVTo
    /// state with Slingshot's hooks.
    pub fn fork_spd(&self, summaries: bool) -> QvtoAdjuster {
        self.fork(SlingshotHooks::new(summaries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_pick_units() {
        let f = SpdVariant::fixed();
        let s = SpdVariant::slingshot();
        for (q, qn, _) in FIXED {
            assert!(f.text(qn).unwrap().contains("PATCHED by SimOxide"), "{q}");
            assert!(!s.text(qn).unwrap().contains("PATCHED"), "{q}");
            assert!(s.text(qn).unwrap().contains("Licence pending confirmation"));
        }
        let only_q6 = SpdVariant {
            q6: true,
            ..SpdVariant::fixed()
        };
        assert!(
            !only_q6
                .text("spd.common.ResourceEnvironmentTransformation")
                .unwrap()
                .contains("PATCHED")
        );
        assert!(
            only_q6
                .text("spd.common.AdjustmentCalculator")
                .unwrap()
                .contains("PATCHED")
        );
        for (qn, _, text) in TRANSFORMATIONS {
            assert!(text.contains("Licence pending confirmation"), "{qn}");
        }
    }

    #[test]
    fn every_variant_compiles_once_per_core() {
        let core = QvtoCore::new().unwrap();
        for v in [SpdVariant::fixed(), SpdVariant::slingshot()] {
            let a = main_transformation(&core, v).unwrap();
            let b = main_transformation(&core, v).unwrap();
            assert!(a.same(&b));
            assert_eq!(a.name(), "MainTransformation");
            assert_eq!(a.params().len(), 7);
        }
    }
}
