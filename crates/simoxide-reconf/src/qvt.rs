//! The QVTo engine on a run's model graph, through the `qvto-simoxide` adapter: the only module
//! of the host that uses the adapter's [`Session`] and [`SimStore`] (with [`crate::Edit`], whose
//! public `store()` is the adapter's store).
//!
//! - [`Template`]: the session every run's session is cloned from (the registry with SimOxide's
//!   metamodels and the metamodels loaded at set-up, the metamodel objects for `eClass()`). It
//!   never sees a graph.
//! - [`RunSession`]: a run's session (resource table, models of other metamodels such as the
//!   PRM, mirrors, journal, id generator), a clone of the template, plain data (`Clone`).
//! - [`RunSession::run`]: one transformation run on a graph, with bindings and natives, the
//!   journal on.
//!
//! When the merge's steps M4-M6 put QVTo on the shared EMF graph, this module is what changes:
//! the template becomes a registry (`Arc<MetaRegistry>`) and a graph prototype, the run session
//! the graph's journal and id policy, and the store the graph itself.

use crate::host::{Bind, InvokeHooks, ModelRef, NsBinding, ParamDir, ParamInfo};
use qvto::blackbox::{NativeError, Natives, palladio};
use qvto_model::{Datum, MetaDerivation, MetaOperation, MetaRegistry, ModelStore, ResourceId};
use qvto_simoxide::{IdPolicy, Journal, Session, SimStore};
use simoxide_model::raw::Graph;
use std::path::Path;
use std::rc::Rc;

/// SimuLizar's runtime measurement metamodel (the PRM) namespace.
pub(crate) const PRM_NS: &str = "http://simulizar.palladiosimulator.org/RuntimeMeasurement/1.0";

/// The metamodels Palladio's analyzers register besides SimOxide's tables, by location: the
/// feature model and feature configuration (PCM 5.2.2 product; SimuLizar's rules commonly
/// declare them as model types), the analyzer framework's completions (Slingshot's SPD
/// transformations use them) and SimuLizar's runtime measurement model (the PRM).
pub(crate) const PALLADIO_METAMODELS: [(&str, &str); 4] = [
    (
        "platform:/plugin/de.uka.ipd.sdq.featuremodel/model/featuremodel.ecore",
        include_str!("../data/featuremodel.ecore"),
    ),
    (
        "platform:/plugin/de.uka.ipd.sdq.featureconfig/model/featureconfig.ecore",
        include_str!("../data/featureconfig.ecore"),
    ),
    (
        "platform:/plugin/org.palladiosimulator.analyzer.completions/model/completions.ecore",
        include_str!("../data/completions.ecore"),
    ),
    (
        "platform:/plugin/org.palladiosimulator.simulizar.runtimemeasurement/model/runtimemeasurement.ecore",
        include_str!("../data/runtimemeasurement.ecore"),
    ),
];

/// The shared part of the QVTo state (see the module documentation).
pub(crate) struct Template {
    session: Session,
}

impl Template {
    /// SimOxide's metamodels plus `metamodels` (`.ecore` text by location), in order.
    pub(crate) fn new(metamodels: &[(String, String)]) -> Result<Template, String> {
        let mut session = Session::new();
        // `.ecore` files refer to Ecore by its plug-in location
        session.resource_set_mut().add_alias(
            "platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore",
            "http://www.eclipse.org/emf/2002/Ecore",
        );
        for (location, text) in metamodels {
            session
                .load_metamodel_text(location, text)
                .map_err(|e| format!("metamodel {location}: {e}"))?;
        }
        Ok(Template { session })
    }

    pub(crate) fn meta(&self) -> &MetaRegistry {
        self.session.meta()
    }

    pub(crate) fn operations(&self) -> Vec<MetaOperation> {
        self.session.operations()
    }

    pub(crate) fn derivations(&self) -> Vec<MetaDerivation> {
        self.session.derivations()
    }

    /// A run's session: a clone of the template with the run's id policy.
    pub(crate) fn instantiate(&self, ids: IdPolicy) -> RunSession {
        let mut session = self.session.clone();
        session.set_id_policy(ids);
        RunSession { session }
    }
}

/// What a transformation run reports (before its changes are applied).
pub(crate) struct RawOutcome {
    pub severity: i32,
    pub code: i32,
    pub message: String,
    pub log: String,
    pub journal: Journal,
    /// Per model parameter: the roots of its extent after the run (`out`/`inout`; store
    /// handles), `None` for `in` parameters.
    pub extents: Vec<Option<Vec<u32>>>,
}

/// The natives every run gets.
pub(crate) struct NativeSpec<'a> {
    /// `getSimulationTime()`, in seconds.
    pub now_s: f64,
    /// Implementation keys of the `ProfilesLibrary` operations (stubs that throw).
    pub profile_keys: &'a [String],
}

/// A run's QVTo state (see the module documentation).
#[derive(Clone)]
pub(crate) struct RunSession {
    session: Session,
}

impl RunSession {
    /// Loads a model of another metamodel (e.g. a PRM file) next to the graph, linking its
    /// references into the graph.
    pub(crate) fn load_model(&mut self, g: &mut Graph, path: &Path) -> Result<ModelRef, String> {
        let mut store = self.session.store(g);
        store.load_model(path).map(|r| ModelRef::Loaded(r.0))
    }

    /// A new, empty resource for objects of other metamodels.
    pub(crate) fn create_model(&mut self, g: &mut Graph, uri: &str) -> ModelRef {
        ModelRef::Loaded(self.session.store(g).create_model_resource(uri).0)
    }

    /// Runs `f` on the store over `g` with the journal on; returns its result and the journal.
    pub(crate) fn with_store<T>(
        &mut self,
        g: &mut Graph,
        f: impl FnOnce(&mut SimStore<'_>) -> T,
    ) -> (T, Journal) {
        self.session.start_journal();
        let t = {
            let mut store = self.session.store(g);
            f(&mut store)
        };
        let j = self.session.stop_journal().unwrap_or_default();
        (t, j)
    }

    /// Runs `f` on copies of this session and of `g`: nothing it does reaches the run.
    pub(crate) fn scratch<T>(&self, g: &Graph, f: impl FnOnce(&mut SimStore<'_>) -> T) -> T {
        let mut session = self.session.clone();
        session.stop_journal();
        let mut g = g.clone();
        let mut store = session.store(&mut g);
        f(&mut store)
    }

    /// Runs `f` on the store over `g` with the journal off (a scratch copy).
    pub(crate) fn scratch_in_place<T>(
        &mut self,
        g: &mut Graph,
        f: impl FnOnce(&mut SimStore<'_>) -> T,
    ) -> T {
        self.session.stop_journal();
        let mut store = self.session.store(g);
        f(&mut store)
    }

    /// Runs `program` on `g`: binds the model parameters, registers the natives, runs with the
    /// journal on. A binding error stops before the run (nothing changed).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        &mut self,
        g: &mut Graph,
        program: &qvto::Program,
        params: &[ParamInfo],
        binds: &[Bind],
        opts: qvto::RunOptions,
        natives: &NativeSpec<'_>,
        hooks: InvokeHooks<'_>,
    ) -> Result<RawOutcome, String> {
        let mut store = self.session.store(g);
        store.session().start_journal();
        if let Some(before) = hooks.before {
            before(&mut store);
        }
        let args = match bind(&store, params, binds) {
            Ok(a) => a,
            Err(e) => {
                store.session().stop_journal();
                return Err(e);
            }
        };
        let mut opts = opts;
        if let Some(ids) = hooks.ids {
            opts.id_source = Some(ids(&store));
        }
        let mut n: Natives<SimStore<'_>> = Natives::new();
        let now = natives.now_s;
        palladio::register_simulation_time(&mut n, move || now);
        for key in natives.profile_keys {
            let name = key.clone();
            n.op(key, move |_, _| {
                Err(NativeError::exception(
                    "java.lang.UnsupportedOperationException",
                    format!(
                        "{name}: stereotype applications (EMF Profiles) are not supported by SimOxide"
                    ),
                ))
            });
        }
        let (out, log) = {
            let mut vm = qvto::Vm::new(program, &mut store, opts);
            vm.set_natives(Rc::new(n));
            let out = vm.run_transformation(args);
            (out, std::mem::take(&mut vm.log))
        };
        let journal = store.session().stop_journal().unwrap_or_default();
        let extents = out
            .extents
            .iter()
            .map(|e| {
                e.as_ref().map(|roots| {
                    roots
                        .iter()
                        .filter(|o| !o.is_local())
                        .map(|o| o.index())
                        .collect()
                })
            })
            .collect();
        Ok(RawOutcome {
            severity: out.severity,
            code: out.code,
            message: out.message,
            log,
            journal,
            extents,
        })
    }
}

/// The root objects per model parameter.
fn bind(
    store: &SimStore<'_>,
    params: &[ParamInfo],
    binds: &[Bind],
) -> Result<Vec<Vec<u32>>, String> {
    let mut args = Vec::with_capacity(params.len());
    for (p, b) in params.iter().zip(binds) {
        if p.direction == ParamDir::Out {
            args.push(Vec::new());
            continue;
        }
        let roots = match b {
            Bind::Empty => Vec::new(),
            Bind::Models(models) => {
                let mut v = Vec::new();
                for m in models {
                    let r = resolve(store, m)
                        .ok_or_else(|| format!("parameter {}: no model {m:?}", p.name))?;
                    v.extend_from_slice(store.roots(r));
                }
                v
            }
            Bind::ByNsUri(how) => by_ns_uri(store, p, *how),
        };
        if roots.is_empty() && !matches!(b, Bind::Empty) {
            // SimuLizar's and Slingshot's executors throw (IllegalStateException)
            return Err(format!(
                "no model in the model cache for parameter {} ({})",
                p.name,
                p.ns_uris.join(", ")
            ));
        }
        args.push(roots);
    }
    Ok(args)
}

/// The store resource a [`ModelRef`] designates.
fn resolve(store: &SimStore<'_>, m: &ModelRef) -> Option<ResourceId> {
    match m {
        ModelRef::Graph(r) => store.graph_resource(*r),
        ModelRef::Loaded(i) => ((*i as usize) < store.resource_count()).then_some(ResourceId(*i)),
        ModelRef::Uri(u) => store.resource_by_uri(u).or_else(|| {
            let suffix = format!("/{u}");
            let mut found = (0..store.resource_count() as u32)
                .map(ResourceId)
                .filter(|r| store.resource_uri(*r).ends_with(&suffix));
            let r = found.next()?;
            found.next().is_none().then_some(r)
        }),
    }
}

/// The models whose metamodel is the parameter's, as SimuLizar's or Slingshot's
/// `QVToModelCache` collects them (in resource order: the graph's resources, then the models
/// of other metamodels; the Java caches iterate a `HashSet`).
fn by_ns_uri(store: &SimStore<'_>, p: &ParamInfo, how: NsBinding) -> Vec<u32> {
    let meta = store.meta();
    let matches = |o: u32| {
        let pkg = meta.class(store.class_of(o)).package;
        meta.package(pkg)
            .ns_uri
            .as_deref()
            .is_some_and(|ns| p.ns_uris.iter().any(|u| u == ns))
    };
    let prm_model = meta
        .package_by_ns_uri(PRM_NS)
        .and_then(|pk| meta.class_by_name(pk, "RuntimeMeasurementModel"));
    let prm_measurements = prm_model.and_then(|c| meta.feature_by_name(c, "measurements"));
    let mut out = Vec::new();
    for r in 0..store.resource_count() as u32 {
        let r = ResourceId(r);
        let roots = store.roots(r);
        match how {
            NsBinding::SimuLizar => {
                let Some(&first) = roots.first() else {
                    continue;
                };
                if !matches(first) {
                    continue;
                }
                // the PRM's measurements go to the extent, not the model (SimuLizar's
                // CREATE_NON_EMPTY_MODEL_ELEMENTS_SWITCH)
                if Some(store.class_of(first)) == prm_model
                    && let Some(f) = prm_measurements
                {
                    for i in 0..store.len(first, f) {
                        if let Datum::Obj(m) = store.get_at(first, f, i) {
                            out.push(m);
                        }
                    }
                } else {
                    out.push(first);
                }
            }
            NsBinding::Slingshot => {
                if store.resource_uri(r) == "pathmap://PCM_MODELS/PrimitiveTypes.repository" {
                    continue;
                }
                out.extend(roots.iter().copied().filter(|&o| matches(o)));
            }
        }
    }
    out
}
