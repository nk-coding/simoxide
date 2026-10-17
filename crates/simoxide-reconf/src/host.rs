//! The QVTo host (`DESIGN-SPD.md` 5.5): QVTo transformations run on a simulation run's model,
//! their journal goes through the change path ([`simoxide_sim::Simulation::apply_changes`]).
//!
//! - [`QvtoCore`] is the shared, immutable part: the metamodel registry, the metamodel objects
//!   and the metamodels loaded at set-up (SimuLizar's runtime measurement metamodel by
//!   default). It is `Send + Sync` and cheap to clone (`Arc`): one per process, used by every
//!   run and batch thread. [`QvtoCore::compile`] compiles a transformation once against it
//!   ([`Transformation`], also shared).
//! - [`QvtoRun`] is a run's QVTo state. It is created empty with the run and filled on the
//!   first use (the session is cloned from the core's, about 1 ms; building one takes 2.6 ms:
//!   too much for runs that never reconfigure). It is plain data: a fork of the run clones it
//!   (copy on write: the fork copies the session when it first uses it), so new object ids
//!   (the seeded UUID generator) continue identically in both.
//! - [`QvtoRun::invoke`] runs a transformation on the run's model graph: binds the model
//!   parameters ([`Bind`]), binds `getSimulationTime()` to the run's clock, collects the
//!   journal and applies its change set to the simulation. [`crate::RuleSet`] runs SimuLizar's
//!   rule folders, [`crate::QvtoAdjuster`] SPD adjustments.
//!
//! The host is generic over the simulator's compat mode and flavour. QVTo's own mode
//! ([`QvtMode`]): the default build has only qvto-rs's fast mode (deterministic insertion
//! order, EPL); builds with the feature `compat` (on through `exact`) run Eclipse's compat
//! mode by default.

use crate::change_set;
use crate::qvt::{self, NativeSpec, RawOutcome, Template};
use qvto_emf::{IdPolicy, Journal, ObjId};
use qvto_model::ModelStore;
use qvto_runtime::Mode;
use qvto_runtime::identity::IdentityHashSource;
use qvto_sema::ir::Direction;
use simoxide_model::raw::{Graph, ResId};
use simoxide_sim::{ChangeError, ChangeHost, ChangeReport, SimError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// QVTo's evaluation mode for the transformations of a run.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum QvtMode {
    /// [`QvtMode::Compat`] in builds with the feature `compat` (or `exact`), else
    /// [`QvtMode::Fast`].
    #[default]
    Auto,
    /// qvto-rs's fast mode: Eclipse's semantics, hash-based collections in deterministic
    /// insertion order (`->any`, `Set` iteration follow it: a documented deviation from
    /// Eclipse's Java `HashMap` and identity-hash orders). Always available (EPL).
    Fast,
    /// Eclipse QVTo's compat mode: Java `HashMap` orders of sets, bags and dictionaries, so that
    /// the models equal the Java runner's. Needs the feature `compat` (no licence covers such
    /// builds, LICENSES section 7).
    Compat,
    /// The Slingshot-exact mode's rule: Eclipse's semantics with QVTo's and OCL's hash
    /// collections insertion-ordered, as the `det` oracle rewrites them (qvto-rs's ordered mode,
    /// `DESIGN-SPD.md` Q2). Always available (EPL).
    InsertionOrderSets,
}

impl QvtMode {
    /// The engine mode this build runs for `self`.
    pub fn resolve(self) -> Result<Mode, ReconfError> {
        resolve_mode(self, Mode::Compat.available())
    }
}

/// [`QvtMode::resolve`] for a build that has (or has not) the compat mode.
pub fn resolve_mode(m: QvtMode, compat_built: bool) -> Result<Mode, ReconfError> {
    match m {
        QvtMode::Auto if compat_built => Ok(Mode::Compat),
        QvtMode::Auto | QvtMode::Fast => Ok(Mode::Fast),
        QvtMode::Compat if compat_built => Ok(Mode::Compat),
        QvtMode::Compat => Err(ReconfError::ModeUnavailable(
            "QVTo compat mode is not compiled in: build simoxide-reconf with the feature `compat` \
             or `exact` (no licence covers such builds, see LICENSES section 7)"
                .into(),
        )),
        QvtMode::InsertionOrderSets => Ok(Mode::Ordered),
    }
}

/// Errors of the host.
#[derive(Debug)]
pub enum ReconfError {
    /// The requested QVTo mode is not compiled into this build ([`QvtMode`]).
    ModeUnavailable(String),
    /// A metamodel could not be loaded into the core.
    Metamodel(String),
    /// A transformation does not compile (unit and messages).
    Compile { unit: String, errors: Vec<String> },
    /// A model could not be loaded.
    Load(String),
    /// A model parameter could not be bound (as SimuLizar's and Slingshot's executors, which
    /// throw `IllegalStateException` when the model cache has no model for it). Nothing
    /// changed.
    Binding(String),
    /// The transformation was compiled by another core.
    WrongCore,
    /// The transformation ran, its changes were not applied: the change path refused them
    /// ([`ChangeError::Invalid`], [`ChangeError::Unsupported`], [`ChangeError::State`], or
    /// [`ChangeError::Abort`] under [`AbortPolicy::Reject`]). The run is unchanged.
    Rejected {
        outcome: Box<Outcome>,
        error: ChangeError,
    },
    /// The change set makes SimuLizar abort ([`ChangeError::Abort`] under
    /// [`AbortPolicy::FailRun`]): the run has failed with `error`.
    RunFailed {
        outcome: Box<Outcome>,
        cause: ChangeError,
        error: SimError,
    },
}

impl std::fmt::Display for ReconfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReconfError::ModeUnavailable(m) => write!(f, "mode unavailable: {m}"),
            ReconfError::Metamodel(m) => write!(f, "metamodel: {m}"),
            ReconfError::Compile { unit, errors } => {
                write!(f, "{unit} does not compile: {}", errors.join("; "))
            }
            ReconfError::Load(m) => write!(f, "load: {m}"),
            ReconfError::Binding(m) => write!(f, "binding: {m}"),
            ReconfError::WrongCore => f.write_str("the transformation belongs to another core"),
            ReconfError::Rejected { outcome, error } => {
                write!(
                    f,
                    "{}: changes not applied: {error}",
                    outcome.transformation
                )
            }
            ReconfError::RunFailed { outcome, error, .. } => {
                write!(f, "{}: the run failed: {error}", outcome.transformation)
            }
        }
    }
}

impl std::error::Error for ReconfError {}

static NEXT_CORE: AtomicU64 = AtomicU64::new(1);

/// The shared, immutable QVTo state (see the module documentation).
#[derive(Clone)]
pub struct QvtoCore(Arc<CoreInner>);

struct CoreInner {
    id: u64,
    /// What a run's graph gets on its first QVTo use (registry, metamodel objects).
    template: Template,
    meta: qvto_model::MetaRegistry,
    operations: Vec<qvto_model::MetaOperation>,
    derivations: Vec<qvto_model::MetaDerivation>,
    /// Implementation keys of Palladio's `ProfilesLibrary` operations (stubs).
    profile_keys: Vec<String>,
    /// Transformations compiled once per core ([`QvtoCore::compiled`]).
    memo: std::sync::Mutex<Vec<(String, Transformation)>>,
}

/// Builds a [`QvtoCore`].
#[derive(Clone, Debug)]
pub struct CoreBuilder {
    palladio: bool,
    metamodels: Vec<(String, String)>,
}

impl CoreBuilder {
    /// Without the metamodels Palladio's analyzers register besides SimOxide's tables (loaded
    /// by default, [`palladio_metamodels`]).
    pub fn without_palladio_metamodels(mut self) -> Self {
        self.palladio = false;
        self
    }

    /// A metamodel from `.ecore` text, registered under `location` (its `platform:/plugin`
    /// URI, which other `.ecore` files and models refer to). Metamodels of SimOxide's tables
    /// (PCM, SPD, monitors, ...) are present already.
    pub fn metamodel_text(mut self, location: &str, text: &str) -> Self {
        self.metamodels
            .push((location.to_string(), text.to_string()));
        self
    }

    pub fn build(self) -> Result<QvtoCore, ReconfError> {
        let mut mms = Vec::new();
        if self.palladio {
            for (location, text) in qvt::PALLADIO_METAMODELS {
                mms.push((location.to_string(), text.to_string()));
            }
        }
        mms.extend(self.metamodels);
        let template = Template::new(&mms).map_err(ReconfError::Metamodel)?;
        let profile_keys = palladio_profile_keys();
        Ok(QvtoCore(Arc::new(CoreInner {
            id: NEXT_CORE.fetch_add(1, Ordering::Relaxed),
            meta: template.meta().clone(),
            operations: template.operations(),
            derivations: template.derivations(),
            template,
            profile_keys,
            memo: std::sync::Mutex::new(Vec::new()),
        })))
    }
}

/// The metamodels a core loads by default besides SimOxide's tables, as (location, `.ecore`
/// text), in loading order: Palladio's feature model and feature configuration, the analyzer
/// framework's completions (Slingshot's SPD transformations use them) and SimuLizar's runtime
/// measurement model (the PRM).
pub fn palladio_metamodels() -> &'static [(&'static str, &'static str)] {
    &qvt::PALLADIO_METAMODELS
}

/// The implementation keys of the `ProfilesLibrary` units of Palladio's catalog.
fn palladio_profile_keys() -> Vec<String> {
    let cat = qvto::blackbox::palladio::catalog();
    let mut keys: Vec<String> = cat
        .envs
        .values()
        .flatten()
        .filter(|u| u.qualified_name.ends_with(".ProfilesLibrary"))
        .flat_map(|u| u.unit.modules.iter())
        .flat_map(|m| m.ops.iter().map(|o| o.java_method.clone()))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Where a transformation's imports are found.
#[derive(Clone, Debug, Default)]
pub enum Imports {
    /// Next to the importing file (SimuLizar's rule folders).
    #[default]
    Folder,
    /// Below these source roots, by qualified name (`pcm.helpers.Commons` is
    /// `<root>/pcm/helpers/Commons.qvto`; Slingshot's transformation bundle).
    Roots(Vec<PathBuf>),
}

impl QvtoCore {
    /// SimOxide's metamodels and the metamodels Palladio's analyzers register besides them
    /// ([`palladio_metamodels`]).
    pub fn new() -> Result<QvtoCore, ReconfError> {
        QvtoCore::builder().build()
    }

    pub fn builder() -> CoreBuilder {
        CoreBuilder {
            palladio: true,
            metamodels: Vec::new(),
        }
    }

    /// The metamodel registry the transformations are compiled against.
    pub fn meta(&self) -> &qvto_model::MetaRegistry {
        &self.0.meta
    }

    /// The `EOperation`s of the registered metamodels (for compiling against [`QvtoCore::meta`]
    /// outside the host).
    pub fn operations(&self) -> &[qvto_model::MetaOperation] {
        &self.0.operations
    }

    /// The OCL derivations of the registered metamodels' features.
    pub fn derivations(&self) -> &[qvto_model::MetaDerivation] {
        &self.0.derivations
    }

    /// Compiles the transformation in `path` (about 1 ms per SimuLizar rule).
    pub fn compile_file(
        &self,
        path: &Path,
        imports: &Imports,
    ) -> Result<Transformation, ReconfError> {
        let files = match imports {
            Imports::Folder => qvto::units::FsResolver::folder(),
            Imports::Roots(roots) => qvto::units::FsResolver {
                bundles: roots
                    .iter()
                    .map(|r| qvto::units::Bundle {
                        root: r.canonicalize().unwrap_or_else(|_| r.clone()),
                        containers: vec![String::new()],
                    })
                    .collect(),
                from_folder: false,
            },
        };
        let main = files
            .unit_source(path)
            .map_err(|e| ReconfError::Load(format!("{}: {e}", path.display())))?;
        let mut t = self.compile(main, &files)?;
        if let Some(inner) = Arc::get_mut(&mut t.0) {
            inner.path = Some(path.to_path_buf());
        }
        Ok(t)
    }

    /// Compiles `main` with its imports from `resolver` (e.g. transformations embedded in the
    /// binary). Palladio's blackbox libraries are registered: `SimulationStateLibrary`
    /// (`getSimulationTime`) and both `ProfilesLibrary` units (stubs that throw when called).
    pub fn compile(
        &self,
        main: qvto::UnitSource,
        resolver: &dyn qvto::UnitResolver,
    ) -> Result<Transformation, ReconfError> {
        let cat = qvto::blackbox::palladio::catalog();
        let mut registered = Vec::new();
        for env in ["simulizar", "architecturaltemplates"] {
            registered.extend(cat.units(env).iter().cloned());
        }
        let bb = qvto::blackbox::BlackboxResolver {
            inner: resolver,
            registered,
            classpath: Vec::new(),
        };
        let unit = main.file_name.clone();
        let compiled = qvto::compile_with_metamodel(
            main,
            &bb,
            self.0.meta.clone(),
            &self.0.operations,
            &self.0.derivations,
        );
        let mut errors = Vec::new();
        for u in &compiled.units {
            for p in u.errors() {
                errors.push(format!(
                    "{}:{}: {}",
                    u.source.file_name,
                    p.line(&u.lines),
                    p.message
                ));
            }
        }
        if let Some(e) = &compiled.bytecode_error {
            errors.push(e.clone());
        }
        let Some(program) = compiled.program else {
            return Err(ReconfError::Compile { unit, errors });
        };
        let ir = &program.ir;
        let main_module = &ir.modules[ir.main_module.index()];
        let params = main_module
            .model_params
            .iter()
            .map(|mp| {
                let mt = &ir.model_types[mp.model_type.index()];
                ParamInfo {
                    name: mp.name.clone(),
                    direction: match mp.direction {
                        Direction::In => ParamDir::In,
                        Direction::InOut => ParamDir::InOut,
                        Direction::Out => ParamDir::Out,
                    },
                    ns_uris: mt
                        .packages
                        .iter()
                        .filter_map(|p| ir.meta.package(*p).ns_uri.as_deref().map(String::from))
                        .collect(),
                }
            })
            .collect();
        Ok(Transformation(Arc::new(TransInner {
            core: self.0.id,
            name: main_module.name.clone(),
            unit,
            path: None,
            program,
            params,
        })))
    }

    /// The transformation compiled under `key` by this core: `compile` runs on the first
    /// request (e.g. the embedded SPD transformations, `crate::spd`), later requests share its
    /// result. A failed compilation is not kept.
    pub fn compiled(
        &self,
        key: &str,
        compile: impl FnOnce(&QvtoCore) -> Result<Transformation, ReconfError>,
    ) -> Result<Transformation, ReconfError> {
        let mut memo = self.0.memo.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, t)) = memo.iter().find(|(k, _)| k == key) {
            return Ok(t.clone());
        }
        let t = compile(self)?;
        memo.push((key.to_string(), t.clone()));
        Ok(t)
    }

    /// Gives a model graph the QVTo state of this core, once (the registry with the core's
    /// metamodels, their metamodel objects for `eClass()`, the id policy of new objects): for
    /// running the core's transformations on a graph outside a simulation. A run's graph gets
    /// it on its first QVTo use.
    pub fn attach(&self, g: &mut Graph, ids: IdPolicy) {
        self.0.template.attach(g, ids);
    }

    /// Loads a model of another metamodel (e.g. a PRM file) into a graph with the QVTo state
    /// ([`QvtoCore::attach`]), its references to the graph's documents resolved.
    pub fn load_model(&self, g: &mut Graph, path: &Path) -> Result<ResId, ReconfError> {
        self.0
            .template
            .load_model(g, path)
            .map_err(ReconfError::Load)
    }
}

/// Direction of a model parameter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParamDir {
    In,
    InOut,
    Out,
}

/// A model parameter of a transformation.
#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub name: String,
    pub direction: ParamDir,
    /// The namespace URIs of its model type's metamodels.
    pub ns_uris: Vec<String>,
}

/// A compiled transformation (shared: `Arc`, `Send + Sync`).
#[derive(Clone)]
pub struct Transformation(Arc<TransInner>);

struct TransInner {
    core: u64,
    name: String,
    unit: String,
    path: Option<PathBuf>,
    program: qvto::Program,
    params: Vec<ParamInfo>,
}

impl Transformation {
    /// The transformation's name (`transformation <name>(...)`).
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The main unit's file name.
    pub fn unit(&self) -> &str {
        &self.0.unit
    }

    pub fn path(&self) -> Option<&Path> {
        self.0.path.as_deref()
    }

    pub fn params(&self) -> &[ParamInfo] {
        &self.0.params
    }

    /// Both are the same compilation (clones of one [`Transformation`]).
    pub fn same(&self, other: &Transformation) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Transformation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Transformation({} in {})", self.0.name, self.0.unit)
    }
}

/// How SimuLizar's or Slingshot's `QVToModelCache` collects the models of a parameter's
/// metamodel ([`Bind::ByNsUri`]).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum NsBinding {
    /// SimuLizar: the first root of each model whose metamodel is the parameter's; for the
    /// runtime measurement model (PRM), its measurements.
    #[default]
    SimuLizar,
    /// Slingshot: every root of each such model, except the bundled `PrimitiveTypes`
    /// repository.
    Slingshot,
}

/// A model of the run: a resource of the model graph, a model of another metamodel loaded
/// into the run's QVTo state ([`QvtoRun::load_model`], [`QvtoRun::create_model`]), or a
/// resource by URI (exact, or the unique resource whose URI ends with `/<uri>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelRef {
    Graph(ResId),
    /// A model loaded or created through the host (a resource of the run's graph as well).
    Loaded(ResId),
    Uri(String),
}

/// What a model parameter is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bind {
    /// The models whose metamodel's namespace URI is one of the parameter's model type (as
    /// SimuLizar and Slingshot bind; [`NsBinding`]), in resource order: the graph's resources,
    /// then the models of other metamodels (the Java caches iterate a `HashSet`: a documented
    /// deviation when several models match).
    ByNsUri(NsBinding),
    /// The roots of these models, in order.
    Models(Vec<ModelRef>),
    /// An empty extent (`out` parameters are always bound to one).
    Empty,
}

impl Default for Bind {
    fn default() -> Self {
        Bind::ByNsUri(NsBinding::SimuLizar)
    }
}

/// The arguments of a transformation run.
#[derive(Clone, Debug, Default)]
pub struct Invocation {
    /// Bindings by parameter name; the other parameters get `default`.
    pub binds: Vec<(String, Bind)>,
    pub default: Bind,
    /// Configuration properties (`configuration property keepModeling : Boolean;`), by name,
    /// as strings converted to the property's type.
    pub config: Vec<(String, String)>,
}

impl Invocation {
    /// Every parameter bound by namespace URI, SimuLizar's way.
    pub fn simulizar() -> Invocation {
        Invocation::default()
    }

    /// Every parameter bound by namespace URI, Slingshot's way.
    pub fn slingshot() -> Invocation {
        Invocation {
            default: Bind::ByNsUri(NsBinding::Slingshot),
            ..Invocation::default()
        }
    }

    pub fn bind(mut self, param: &str, b: Bind) -> Self {
        self.binds.push((param.to_string(), b));
        self
    }

    pub fn config(mut self, name: &str, value: &str) -> Self {
        self.config.push((name.to_string(), value.to_string()));
        self
    }

    fn binds_for(&self, params: &[ParamInfo]) -> Vec<Bind> {
        params
            .iter()
            .map(|p| {
                self.binds
                    .iter()
                    .rev()
                    .find(|(n, _)| *n == p.name)
                    .map_or_else(|| self.default.clone(), |(_, b)| b.clone())
            })
            .collect()
    }
}

/// Makes the identity-hash source of a compat-mode run from the store before the run.
pub type IdSourceFn<'h> = dyn FnMut(&Graph) -> Box<dyn IdentityHashSource> + 'h;

/// Hooks of a transformation run ([`QvtoRun::invoke_with`]).
#[derive(Default)]
pub struct InvokeHooks<'h> {
    /// Edits of the models before the run, recorded in the run's journal and applied with its
    /// changes (e.g. SPD's `Configuration.enactedPolicy`); the parameters are bound after it.
    pub before: Option<&'h mut dyn FnMut(&mut Graph)>,
    /// The identity hashes of the compat mode, given the store before the run (replaying a Java
    /// run's identity hashes for exact comparisons). Default: HotSpot's generator.
    pub ids: Option<&'h mut IdSourceFn<'h>>,
}

/// What a [`ChangeError::Abort`] does to the run.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum AbortPolicy {
    /// SimuLizar: the syncer throws inside the reconfiguration process and the simulation
    /// aborts. The run fails ([`simoxide_sim::Simulation::fail_run`], [`ReconfError::RunFailed`]).
    #[default]
    FailRun,
    /// The changes are not applied and the run continues unchanged ([`ReconfError::Rejected`]).
    Reject,
}

/// What happens to the changes of a transformation run that did not succeed (severity
/// `ERROR`, e.g. `assert fatal` or an exception).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FailedRunChanges {
    /// They are applied, as in SimuLizar and Slingshot, which run the transformations on the
    /// models in place: what a rule changed before it failed stays.
    #[default]
    Keep,
    /// They are dropped.
    Discard,
}

/// The configuration of a run's QVTo state.
#[derive(Clone, Debug)]
pub struct RunConfig {
    pub mode: QvtMode,
    /// `Identifier.id` of new PCM objects: seeded EMF-style UUIDs (default; generated PCM code
    /// gives every new object a UUID, reproducibly here), or none (dynamic EMF, as the corpus's
    /// Java runs).
    pub ids: IdPolicy,
    pub on_abort: AbortPolicy,
    pub failed_runs: FailedRunChanges,
    /// QVTo's call-depth limit (`EVALUATION_MAX_STACK_DEPTH`, Eclipse's default 1000).
    pub max_stack_depth: usize,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            mode: QvtMode::Auto,
            ids: IdPolicy::Uuid {
                seed: 0x5EED_0F51_u64,
            },
            on_abort: AbortPolicy::FailRun,
            failed_runs: FailedRunChanges::Keep,
            max_stack_depth: 1000,
        }
    }
}

/// The outcome of a transformation run.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// The transformation's name.
    pub transformation: String,
    /// EMF severity (0 OK, 1 INFO, 2 WARNING, 4 ERROR, 8 CANCEL), code and message.
    pub severity: i32,
    pub code: i32,
    pub message: String,
    /// Severity OK or INFO: SimuLizar's and Slingshot's "applied" (`handleExecutionResult`);
    /// an `assert fatal` makes it false.
    pub succeeded: bool,
    /// The QVTo log (`log(...)` records).
    pub log: String,
    /// Every change the run made, to the graph and to the models of other metamodels.
    pub journal: Journal,
    /// What applying the graph changes did; `None` when the run changed no graph object (or
    /// its changes were discarded, [`FailedRunChanges::Discard`]).
    pub change: Option<ChangeReport>,
    /// Per model parameter: the roots of its extent after the run (`out` and `inout`
    /// parameters; store handles, see [`QvtoRun::with_scratch_store`]), `None` for `in`.
    pub extents: Vec<Option<Vec<ObjId>>>,
    /// Wall time of the transformation run (µs).
    pub run_micros: u64,
}

fn outcome(name: &str, raw: RawOutcome, micros: u64) -> Outcome {
    Outcome {
        transformation: name.to_string(),
        severity: raw.severity,
        code: raw.code,
        message: raw.message,
        succeeded: raw.severity == 0 || raw.severity == 1,
        log: raw.log,
        journal: raw.journal,
        change: None,
        extents: raw.extents,
        run_micros: micros,
    }
}

/// A run's QVTo state (see the module documentation). `Clone` is the state of a fork.
#[derive(Clone)]
pub struct QvtoRun {
    core: QvtoCore,
    cfg: RunConfig,
    mode: Mode,
    pub(crate) state: Option<Box<PerRun>>,
}

#[derive(Clone)]
pub(crate) struct PerRun {
    /// The graph the transformations run on: the simulation's model graph with the QVTo state
    /// (metamodel objects, models of other metamodels such as the PRM, the id generator), and
    /// the simulation state it belongs to. Shared copy on write between the clones of a run's
    /// state (a fork copies it when it first changes it). Whenever only models of other
    /// metamodels changed, the simulation gets it too (`Simulation::set_model_graph`), so that
    /// they live with the run's model (forks, resumed runs, model changes).
    work: Option<(Arc<Graph>, Fp)>,
    pub(crate) invocations: u64,
}

impl QvtoRun {
    /// The state a snapshot keeps of the run's QVTo state in its working graph.
    #[cfg(feature = "snapshot")]
    pub(crate) fn session_state(&self) -> Option<crate::qvt::SessionState> {
        let st = self.state.as_ref()?;
        let g = st.work.as_ref().map(|(g, _)| &**g)?;
        Some(self.core.0.template.save_state(g))
    }

    /// Continues from a saved state on the restored simulation `sim`: its graph gets the QVTo
    /// state back with every id (see `qvt::SessionState`).
    #[cfg(feature = "snapshot")]
    pub(crate) fn restore_session<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &H,
        invocations: u64,
        s: Option<&crate::qvt::SessionState>,
    ) -> Result<(), String> {
        let mut g = sim.model_graph().clone();
        match s {
            Some(s) => self
                .core
                .0
                .template
                .restore_state(&mut g, s, self.cfg.ids)?,
            None => self.core.0.template.attach(&mut g, self.cfg.ids),
        }
        self.state = Some(Box::new(PerRun {
            work: Some((Arc::new(g), fingerprint(sim))),
            invocations,
        }));
        Ok(())
    }
}

/// Which simulation model a working graph belongs to: the model's generation, the compiled
/// model and the model graph (addresses).
type Fp = (u32, usize, usize);

fn fingerprint<H: ChangeHost + ?Sized>(sim: &H) -> Fp {
    (
        sim.model_generation(),
        sim.compiled_now() as *const _ as usize,
        sim.model_graph() as *const _ as usize,
    )
}

impl QvtoRun {
    /// A run's QVTo state, empty until the first use. Fails when `cfg.mode` is not available
    /// in this build.
    pub fn new(core: &QvtoCore, cfg: RunConfig) -> Result<QvtoRun, ReconfError> {
        let mode = cfg.mode.resolve()?;
        Ok(QvtoRun {
            core: core.clone(),
            cfg,
            mode,
            state: None,
        })
    }

    pub fn core(&self) -> &QvtoCore {
        &self.core
    }

    pub fn config(&self) -> &RunConfig {
        &self.cfg
    }

    /// The engine mode the transformations run in.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether the per-run state exists (a transformation ran or a model was loaded).
    pub fn is_started(&self) -> bool {
        self.state.is_some()
    }

    /// Number of transformation runs so far.
    pub fn invocations(&self) -> u64 {
        self.state.as_ref().map_or(0, |s| s.invocations)
    }

    /// The per-run state with a working graph equal to `sim`'s model graph.
    pub(crate) fn prepare<H: ChangeHost + ?Sized>(&mut self, sim: &H) -> &mut PerRun {
        let core = &self.core;
        let ids = self.cfg.ids;
        let st = self.state.get_or_insert_with(|| {
            Box::new(PerRun {
                work: None,
                invocations: 0,
            })
        });
        let fp = fingerprint(sim);
        if st.work.as_ref().is_none_or(|(_, f)| *f != fp) {
            let mut g = sim.model_graph().clone();
            match st.work.take() {
                // A restored run's model holds placeholders where the QVTo state was until the
                // first transformation hands its working graph to the simulation; a model change
                // from outside (a change file at the resume) copies that model: the restored
                // state (id generator, objects of other metamodels) carries over to the copy, or
                // the next transformation would reuse ids.
                #[cfg(feature = "snapshot")]
                Some((old, _))
                    if g.links().package_obj.is_empty() && !old.links().package_obj.is_empty() =>
                {
                    let saved = core.0.template.save_state(&old);
                    if core.0.template.restore_state(&mut g, &saved, ids).is_err() {
                        g = sim.model_graph().clone();
                        core.0.template.attach(&mut g, ids);
                    }
                }
                _ => core.0.template.attach(&mut g, ids),
            }
            st.work = Some((Arc::new(g), fp));
        }
        st
    }

    /// The working graph, unshared (copied when a fork shares it).
    fn work_mut(&mut self) -> &mut Graph {
        let st = self.state.as_mut().expect("per-run state");
        Arc::make_mut(&mut st.work.as_mut().expect("work graph").0)
    }

    /// Hands the working graph to the simulation when only models of other metamodels changed
    /// (see [`PerRun::work`]).
    fn share_work<H: ChangeHost + ?Sized>(&mut self, sim: &mut H) {
        let Some(st) = self.state.as_mut() else {
            return;
        };
        let Some((g, fp)) = st.work.as_mut() else {
            return;
        };
        if sim.set_model_graph(g.clone()).is_ok() {
            *fp = fingerprint(sim);
        } else {
            st.work = None;
        }
    }

    /// Loads a model of another metamodel than SimOxide's (e.g. a runtime measurement model)
    /// into the run's model graph; its references to the PCM documents are resolved. The
    /// returned reference binds it ([`Bind::Models`]). The model lives with the run's model
    /// from then on (forks, model changes).
    pub fn load_model<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &mut H,
        path: &Path,
    ) -> Result<ModelRef, ReconfError> {
        self.prepare(sim);
        let core = self.core.clone();
        let r = core.0.template.load_model(self.work_mut(), path);
        self.share_work(sim);
        r.map(ModelRef::Loaded).map_err(ReconfError::Load)
    }

    /// A new, empty model for objects of other metamodels (e.g. the PRM the reconfiguration
    /// process fills, R1), filled with [`QvtoRun::with_store`].
    pub fn create_model<H: ChangeHost + ?Sized>(&mut self, sim: &mut H, uri: &str) -> ModelRef {
        self.prepare(sim);
        let r = self.work_mut().create_resource(uri);
        self.share_work(sim);
        ModelRef::Loaded(r)
    }

    /// Runs `f` on the run's model graph (any `qvto_model::ModelStore` operation: writing
    /// runtime measurement values, setting SPD's `enactedPolicy`, ...). Changes of PCM objects
    /// are applied to the simulation like a transformation's.
    pub fn with_store<H: ChangeHost + ?Sized, T>(
        &mut self,
        sim: &mut H,
        f: impl FnOnce(&mut Graph) -> T,
    ) -> Result<(T, Option<ChangeReport>), ReconfError> {
        self.prepare(sim);
        let (t, journal) = qvt::with_journal(self.work_mut(), f);
        let raw = RawOutcome {
            severity: 0,
            code: 0,
            message: String::new(),
            log: String::new(),
            journal,
            extents: Vec::new(),
        };
        let out = self.apply(sim, "with_store", raw, 0)?;
        Ok((t, out.change))
    }

    /// Runs `f` on a scratch copy of the run's model graph (with the QVTo state): for reading
    /// (dumps, summaries, diffs) and for trying changes; nothing reaches the run.
    pub fn with_scratch_store<H: ChangeHost + ?Sized, T>(
        &mut self,
        sim: &H,
        f: impl FnOnce(&mut Graph) -> T,
    ) -> T {
        let st = self.prepare(sim);
        let mut g = (*st.work.as_ref().expect("work graph").0).clone();
        f(&mut g)
    }

    /// Runs `t` on the run's model and applies its changes (see the module documentation).
    ///
    /// Errors: a parameter cannot be bound (nothing ran); the change path refuses the changes
    /// (the run is unchanged, [`ReconfError::Rejected`]); SimuLizar would abort (the run has
    /// failed, [`ReconfError::RunFailed`], unless [`AbortPolicy::Reject`]). A transformation
    /// that fails (`assert fatal`, an exception) is no error: [`Outcome::succeeded`] is false.
    pub fn invoke<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &mut H,
        t: &Transformation,
        inv: &Invocation,
    ) -> Result<Outcome, ReconfError> {
        self.invoke_with(sim, t, inv, InvokeHooks::default())
    }

    /// [`QvtoRun::invoke`] with [`InvokeHooks`]: edits before the run in its journal, identity
    /// hashes.
    pub fn invoke_with<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &mut H,
        t: &Transformation,
        inv: &Invocation,
        hooks: InvokeHooks<'_>,
    ) -> Result<Outcome, ReconfError> {
        let (raw, micros) = self.execute(sim, t, inv, hooks, None)?;
        self.apply(sim, &t.0.name, raw, micros)
    }

    /// Runs `t` like [`QvtoRun::invoke_with`] on a scratch copy of the run's model graph, then
    /// `f` on that graph after the run: the outcome (with [`Outcome::change`] `None`) and what
    /// `f` returns; nothing reaches the run. For trying a reconfiguration, and for inspecting a
    /// transformation's result (dumps, diffs).
    pub fn dry_run<H: ChangeHost + ?Sized, T>(
        &mut self,
        sim: &H,
        t: &Transformation,
        inv: &Invocation,
        hooks: InvokeHooks<'_>,
        f: impl FnOnce(&mut Graph, &Outcome) -> T,
    ) -> Result<(Outcome, T), ReconfError> {
        let mut scratch = None;
        let (raw, micros) = self.execute(sim, t, inv, hooks, Some(&mut scratch))?;
        let out = outcome(&t.0.name, raw, micros);
        let mut g: Graph = scratch.expect("scratch graph");
        let r = f(&mut g, &out);
        Ok((out, r))
    }

    /// Runs `t` on the working graph, or on a copy stored in `scratch`.
    fn execute<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &H,
        t: &Transformation,
        inv: &Invocation,
        hooks: InvokeHooks<'_>,
        scratch: Option<&mut Option<Graph>>,
    ) -> Result<(RawOutcome, u64), ReconfError> {
        if t.0.core != self.core.0.id {
            return Err(ReconfError::WrongCore);
        }
        let now_s = sim.now_s();
        let opts = qvto::RunOptions {
            mode: self.mode,
            max_stack_depth: self.cfg.max_stack_depth,
            config: inv
                .config
                .iter()
                .map(|(k, v)| (k.clone(), Some(v.clone())))
                .collect(),
            ..qvto::RunOptions::default()
        };
        let binds = inv.binds_for(&t.0.params);
        let core = self.core.clone();
        let natives = NativeSpec {
            now_s,
            profile_keys: &core.0.profile_keys,
        };
        self.prepare(sim);
        let t0;
        let raw = match scratch {
            None => {
                self.state.as_mut().expect("per-run state").invocations += 1;
                let g = self.work_mut();
                t0 = Instant::now();
                qvt::run(g, &t.0.program, &t.0.params, &binds, opts, &natives, hooks)
            }
            Some(slot) => {
                let st = self.state.as_ref().expect("per-run state");
                let mut g = (*st.work.as_ref().expect("work graph").0).clone();
                t0 = Instant::now();
                let r = qvt::run(
                    &mut g,
                    &t.0.program,
                    &t.0.params,
                    &binds,
                    opts,
                    &natives,
                    hooks,
                );
                *slot = Some(g);
                r
            }
        }
        .map_err(ReconfError::Binding)?;
        Ok((raw, t0.elapsed().as_micros() as u64))
    }

    /// Applies the changes of a run to the simulation (or drops them): the changes of PCM
    /// objects through the change path; changes of other models only, by handing the graph
    /// over.
    fn apply<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &mut H,
        name: &str,
        raw: RawOutcome,
        micros: u64,
    ) -> Result<Outcome, ReconfError> {
        let mut out = outcome(name, raw, micros);
        let succeeded = out.succeeded;
        if out.journal.is_empty() {
            return Ok(out);
        }
        if !succeeded && self.cfg.failed_runs == FailedRunChanges::Discard {
            self.state.as_mut().expect("per-run state").work = None;
            return Ok(out);
        }
        let st = self.state.as_mut().expect("per-run state");
        let (g, _) = st.work.as_ref().expect("work graph");
        let cs = change_set(g, &out.journal);
        if cs.is_empty() {
            self.share_work(sim);
            return Ok(out);
        }
        match sim.apply_changes((**g).clone(), &cs) {
            Ok(rep) => {
                // the simulation's graph equals the working graph now
                if let Some((_, fp)) = st.work.as_mut() {
                    *fp = fingerprint(sim);
                }
                out.change = Some(rep);
                Ok(out)
            }
            Err(error) => {
                st.work = None;
                match error {
                    ChangeError::Abort(_) if self.cfg.on_abort == AbortPolicy::FailRun => {
                        let e = sim.fail_run(format!("reconfiguration by {name}: {error}"));
                        Err(ReconfError::RunFailed {
                            outcome: Box::new(out),
                            cause: error,
                            error: e,
                        })
                    }
                    error => Err(ReconfError::Rejected {
                        outcome: Box::new(out),
                        error,
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_sync<T: Send + Sync>() {}
    fn send<T: Send>() {}

    #[test]
    fn threads() {
        send_sync::<QvtoCore>();
        send_sync::<Transformation>();
        send::<QvtoRun>();
    }

    #[test]
    fn modes() {
        assert_eq!(resolve_mode(QvtMode::Auto, false).unwrap(), Mode::Fast);
        assert_eq!(resolve_mode(QvtMode::Auto, true).unwrap(), Mode::Compat);
        assert_eq!(resolve_mode(QvtMode::Fast, true).unwrap(), Mode::Fast);
        assert_eq!(resolve_mode(QvtMode::Compat, true).unwrap(), Mode::Compat);
        assert!(matches!(
            resolve_mode(QvtMode::Compat, false),
            Err(ReconfError::ModeUnavailable(_))
        ));
        assert_eq!(
            resolve_mode(QvtMode::InsertionOrderSets, false).unwrap(),
            Mode::Ordered
        );
        assert_eq!(QvtMode::Compat.resolve().is_ok(), Mode::Compat.available());
    }
}
