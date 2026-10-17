//! The QVTo engine on a run's model graph: the only module of the host that runs the engine.
//! Since the qvto merge's step M6 the engine runs on the shared EMF graph itself
//! (`simoxide_emf::Graph`, SimOxide's model graph): one graph holds the PCM models, the models
//! of other metamodels (the PRM, SPD), the metamodel objects for `eClass()`, the journal and the
//! id generator.
//!
//! - [`Template`]: what every run's graph gets on its first QVTo use: the registry with
//!   SimOxide's metamodels and the metamodels loaded at set-up, and a prototype graph with their
//!   metamodel objects (appended to the run's graph, `Graph::append_graph`). Shared, immutable.
//! - [`run`]: one transformation run on a graph, with bindings and natives, the journal on.

use crate::host::{Bind, InvokeHooks, ModelRef, NsBinding, ParamDir, ParamInfo};
use qvto::blackbox::{NativeError, Natives, palladio};
use qvto_emf::{IdPolicy, Journal, ObjId, ResourceSet};
use qvto_model::{Datum, MetaDerivation, MetaOperation, MetaRegistry, ModelStore, ResourceId};
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
    /// The prototype graph (registry, Ecore and the metamodel objects; no models) with the URI
    /// configuration for loading models.
    rs: ResourceSet,
}

impl Template {
    /// SimOxide's metamodels plus `metamodels` (`.ecore` text by location), in order.
    pub(crate) fn new(metamodels: &[(String, String)]) -> Result<Template, String> {
        let mut rs = ResourceSet::with_graph(simoxide_model::raw::pcm_graph());
        let meta = rs.graph.meta();
        let ops = meta.static_operations(rs.graph.ecore().package);
        let tops = meta.static_top_packages();
        rs.materialize(&tops, &ops)
            .map_err(|e| format!("metamodel objects of SimOxide's packages: {e}"))?;
        // `.ecore` files refer to Ecore by its plug-in location
        rs.add_alias(
            "platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore",
            "http://www.eclipse.org/emf/2002/Ecore",
        );
        for (location, text) in metamodels {
            let r = rs
                .load_text(location, text)
                .map_err(|e| format!("metamodel {location}: {e}"))?;
            rs.register(r)
                .map_err(|e| format!("metamodel {location}: {e}"))?;
        }
        Ok(Template { rs })
    }

    pub(crate) fn meta(&self) -> &MetaRegistry {
        self.rs.graph.meta()
    }

    pub(crate) fn operations(&self) -> Vec<MetaOperation> {
        qvto_emf::operations::meta_operations(&self.rs.graph)
    }

    pub(crate) fn derivations(&self) -> Vec<MetaDerivation> {
        qvto_emf::operations::meta_derivations(&self.rs.graph)
    }

    /// Gives a model graph the QVTo state, once: the registry and the metamodel objects, and
    /// the id policy of new objects. A graph that has it (e.g. a run's graph after a model
    /// change) keeps its state, its id generator included.
    pub(crate) fn attach(&self, g: &mut Graph, ids: IdPolicy) {
        if g.links().package_obj.is_empty() {
            g.append_graph(&self.rs.graph);
            g.set_id_policy(ids);
        } else if !std::sync::Arc::ptr_eq(g.meta_arc(), self.rs.graph.meta_arc()) {
            g.adopt_meta(self.rs.graph.meta_arc().clone());
        }
    }

    /// Loads a model of another metamodel (e.g. a PRM file) into `g` (with the QVTo state),
    /// resolving its references (`href`s to the graph's PCM documents become references to
    /// their objects).
    pub(crate) fn load_model(&self, g: &mut Graph, path: &Path) -> Result<ResourceId, String> {
        let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
        let uri = qvto_emf::uri::Uri::file(&abs.to_string_lossy()).to_string();
        let empty = g.empty_like();
        let mut rs = self.rs.rebind(std::mem::replace(g, empty));
        let r = match rs.get_resource(&uri, false) {
            Some(r) => Ok(r),
            None => rs.load(&uri),
        };
        if let Ok(r) = r {
            rs.resolve_resource(r);
        }
        *g = rs.graph;
        r
    }
}

/// What a transformation run reports (before its changes are applied).
pub(crate) struct RawOutcome {
    pub severity: i32,
    pub code: i32,
    pub message: String,
    pub log: String,
    pub journal: Journal,
    /// Per model parameter: the roots of its extent after the run (`out`/`inout`), `None` for
    /// `in` parameters.
    pub extents: Vec<Option<Vec<ObjId>>>,
}

/// The natives every run gets.
pub(crate) struct NativeSpec<'a> {
    /// `getSimulationTime()`, in seconds.
    pub now_s: f64,
    /// Implementation keys of the `ProfilesLibrary` operations (stubs that throw).
    pub profile_keys: &'a [String],
}

/// Runs `f` on `g` with the journal on; returns its result and the journal.
pub(crate) fn with_journal<T>(g: &mut Graph, f: impl FnOnce(&mut Graph) -> T) -> (T, Journal) {
    g.start_journal();
    let t = f(g);
    let j = g.stop_journal().unwrap_or_default();
    (t, j)
}

/// Runs `program` on `g`: binds the model parameters, registers the natives, runs with the
/// journal on. A binding error stops before the run (nothing changed but the hook's edits).
pub(crate) fn run(
    g: &mut Graph,
    program: &qvto::Program,
    params: &[ParamInfo],
    binds: &[Bind],
    opts: qvto::RunOptions,
    natives: &NativeSpec<'_>,
    hooks: InvokeHooks<'_>,
) -> Result<RawOutcome, String> {
    g.start_journal();
    if let Some(before) = hooks.before {
        before(g);
    }
    let args = match bind(g, params, binds) {
        Ok(a) => a,
        Err(e) => {
            g.stop_journal();
            return Err(e);
        }
    };
    let mut opts = opts;
    if let Some(ids) = hooks.ids {
        opts.id_source = Some(ids(g));
    }
    let mut n: Natives<Graph> = Natives::new();
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
        let mut vm = qvto::Vm::new(program, g, opts);
        vm.set_natives(Rc::new(n));
        let out = vm.run_transformation(args);
        (out, std::mem::take(&mut vm.log))
    };
    let journal = g.stop_journal().unwrap_or_default();
    let extents = out
        .extents
        .iter()
        .map(|e| {
            e.as_ref().map(|roots| {
                roots
                    .iter()
                    .filter(|o| !o.is_local())
                    .map(|o| ObjId(o.index()))
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

/// The root objects per model parameter.
fn bind(g: &Graph, params: &[ParamInfo], binds: &[Bind]) -> Result<Vec<Vec<ObjId>>, String> {
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
                    let r = resolve(g, m)
                        .ok_or_else(|| format!("parameter {}: no model {m:?}", p.name))?;
                    v.extend_from_slice(g.roots(r));
                }
                v
            }
            Bind::ByNsUri(how) => by_ns_uri(g, p, *how),
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

/// The resource a [`ModelRef`] designates.
fn resolve(g: &Graph, m: &ModelRef) -> Option<ResourceId> {
    match m {
        ModelRef::Graph(r) | ModelRef::Loaded(r) => {
            ((r.0 as usize) < g.resource_count()).then_some(*r)
        }
        ModelRef::Uri(u) => g.resource_by_uri(u).or_else(|| {
            let suffix = format!("/{u}");
            let mut found = (0..g.resource_count() as u32)
                .map(ResourceId)
                .filter(|r| g.resource_uri(*r).ends_with(&suffix));
            let r = found.next()?;
            found.next().is_none().then_some(r)
        }),
    }
}

/// The models whose metamodel is the parameter's, as SimuLizar's or Slingshot's
/// `QVToModelCache` collects them (in resource order: the PCM documents, then the models of
/// other metamodels loaded later; the Java caches iterate a `HashSet`).
fn by_ns_uri(g: &Graph, p: &ParamInfo, how: NsBinding) -> Vec<ObjId> {
    let meta = g.meta();
    let matches = |o: ObjId| {
        let pkg = meta.class(g.class_of(o)).package;
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
    for r in 0..g.resource_count() as u32 {
        let r = ResourceId(r);
        let roots = g.roots(r);
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
                if Some(g.class_of(first)) == prm_model
                    && let Some(f) = prm_measurements
                {
                    for i in 0..g.len(first, f) {
                        if let Datum::Obj(m) = g.get_at(first, f, i) {
                            out.push(m);
                        }
                    }
                } else {
                    out.push(first);
                }
            }
            NsBinding::Slingshot => {
                if g.resource_uri(r) == "pathmap://PCM_MODELS/PrimitiveTypes.repository" {
                    continue;
                }
                out.extend(roots.iter().copied().filter(|&o| matches(o)));
            }
        }
    }
    out
}
