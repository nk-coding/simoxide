//! The run state in a snapshot (`crate::snapshot`; `DESIGN-SPD.md` 5.7, N1): the section
//! `engine` ([`EngineImage`]) and, after model changes, the section `model` ([`ModelImage`]);
//! [`Simulation::snapshot_to`] and [`Simulation::restore_from`].
//!
//! Most of the state is written as it is (serde derives on the engine's types). What is not
//! plain data is converted:
//!
//! * the frames and evaluation proxies (`Rc` graph, shared copy on write) become an arena whose
//!   entries refer to earlier ones; restoring keeps the sharing;
//! * the random source becomes its generator state ([`crate::snapshot::RngImage`]);
//! * maps are written in key order (deterministic bytes);
//! * the configuration without the run-time controls (`Limits::deadline`, `Limits::cancel`);
//! * the run's own model (after model changes): the model graph as XMI documents with the
//!   object table ([`simoxide_model::persist::GraphXmi`]), the build order and the compiled
//!   tables; the typed model is rebuilt from the graph in the build order, the StoEx programs
//!   from their specifications, so every index of the state keeps its meaning.

use super::runmodel::RunModel;
use super::state::{Shape, max_depth};
use super::*;
use crate::rng::Tape;
use crate::snapshot::{
    Encoding, FORMAT, Header, RngImage, RunModelInfo, Section, Snapshot, SnapshotError,
    SnapshotInfo, SnapshotOptions, model_fingerprint,
};
use serde::{Deserialize, Serialize};
use simoxide_model::persist::GraphXmi;
use simoxide_model::raw::ObjId;
use std::rc::Rc;
use std::sync::Arc;

/// Name of the run-state section.
pub const ENGINE: &str = "engine";
/// Name of the section with the run's own model.
pub const MODEL: &str = "model";
/// Name of the section with the run's model graph when it differs from its compiled model's
/// outside SimOxide's metamodels only (`Simulation::set_model_graph`: a QVTo host's state).
pub const GRAPH: &str = "graph";

/// A StoEx value in a frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum ValueImage {
    Int(i32),
    Double(f64),
    Bool(bool),
    Str(String),
    Null,
}

impl From<&Value> for ValueImage {
    fn from(v: &Value) -> Self {
        match v {
            Value::Int(i) => ValueImage::Int(*i),
            Value::Double(d) => ValueImage::Double(*d),
            Value::Bool(b) => ValueImage::Bool(*b),
            Value::Str(s) => ValueImage::Str(s.to_string()),
            Value::Null => ValueImage::Null,
        }
    }
}

impl From<ValueImage> for Value {
    fn from(v: ValueImage) -> Self {
        match v {
            ValueImage::Int(i) => Value::Int(i),
            ValueImage::Double(d) => Value::Double(d),
            ValueImage::Bool(b) => Value::Bool(b),
            ValueImage::Str(s) => Value::Str(s.into()),
            ValueImage::Null => Value::Null,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum BindingImage {
    Val(ValueImage),
    /// Index of a proxy node.
    Proxy(u32),
}

/// A node of the frame arena: entries refer to earlier nodes only.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum NodeImage {
    Frame {
        entries: Vec<(KeyId, BindingImage)>,
        parent: Option<u32>,
    },
    Proxy {
        prog: ProgId,
        frame: Option<u32>,
    },
}

/// Collects the frame graph into an arena in dependency order (parents and proxy frames first).
#[derive(Default)]
struct Arena {
    nodes: Vec<NodeImage>,
    frames: HashMap<*const Frame, u32>,
    proxies: HashMap<*const Proxy, u32>,
}

enum Visit {
    F(FrameRef),
    P(ProxyRef),
}

impl Arena {
    fn frame(&mut self, f: &FrameRef) -> u32 {
        if let Some(&i) = self.frames.get(&Rc::as_ptr(f)) {
            return i;
        }
        // depth first, post order, without recursion (parent chains are as long as the calls)
        let mut stack = vec![(Visit::F(f.clone()), false)];
        while let Some((v, ready)) = stack.pop() {
            match v {
                Visit::F(fr) => {
                    if self.frames.contains_key(&Rc::as_ptr(&fr)) {
                        continue;
                    }
                    if !ready {
                        let deps: Vec<Visit> = fr
                            .parent
                            .iter()
                            .filter(|p| !self.frames.contains_key(&Rc::as_ptr(p)))
                            .map(|p| Visit::F(p.clone()))
                            .chain(fr.entries.iter().filter_map(|(_, b)| match b {
                                Binding::Proxy(p) if !self.proxies.contains_key(&Rc::as_ptr(p)) => {
                                    Some(Visit::P(p.clone()))
                                }
                                _ => None,
                            }))
                            .collect();
                        stack.push((Visit::F(fr), true));
                        stack.extend(deps.into_iter().rev().map(|d| (d, false)));
                        continue;
                    }
                    let entries = fr
                        .entries
                        .iter()
                        .map(|(k, b)| {
                            let b = match b {
                                Binding::Val(v) => BindingImage::Val(v.into()),
                                Binding::Proxy(p) => {
                                    BindingImage::Proxy(self.proxies[&Rc::as_ptr(p)])
                                }
                            };
                            (*k, b)
                        })
                        .collect();
                    let parent = fr.parent.as_ref().map(|p| self.frames[&Rc::as_ptr(p)]);
                    self.frames.insert(Rc::as_ptr(&fr), self.nodes.len() as u32);
                    self.nodes.push(NodeImage::Frame { entries, parent });
                }
                Visit::P(px) => {
                    if self.proxies.contains_key(&Rc::as_ptr(&px)) {
                        continue;
                    }
                    if !ready {
                        let dep = px
                            .frame
                            .as_ref()
                            .filter(|f| !self.frames.contains_key(&Rc::as_ptr(f)))
                            .cloned();
                        stack.push((Visit::P(px), true));
                        if let Some(d) = dep {
                            stack.push((Visit::F(d), false));
                        }
                        continue;
                    }
                    let frame = px.frame.as_ref().map(|f| self.frames[&Rc::as_ptr(f)]);
                    self.proxies
                        .insert(Rc::as_ptr(&px), self.nodes.len() as u32);
                    self.nodes.push(NodeImage::Proxy {
                        prog: px.prog,
                        frame,
                    });
                }
            }
        }
        self.frames[&Rc::as_ptr(f)]
    }
}

/// The frames and proxies of an arena, rebuilt with their sharing.
struct Rebuilt {
    frames: Vec<Option<FrameRef>>,
}

impl Rebuilt {
    fn new(nodes: Vec<NodeImage>) -> Result<Rebuilt, String> {
        let bad = || "damaged frame arena".to_string();
        let mut frames: Vec<Option<FrameRef>> = Vec::with_capacity(nodes.len());
        let mut proxies: Vec<Option<ProxyRef>> = Vec::with_capacity(nodes.len());
        for n in nodes {
            match n {
                NodeImage::Frame { entries, parent } => {
                    let parent = match parent {
                        Some(i) => Some(frames.get(i as usize).cloned().flatten().ok_or_else(bad)?),
                        None => None,
                    };
                    let mut es = Vec::with_capacity(entries.len());
                    for (k, b) in entries {
                        let b = match b {
                            BindingImage::Val(v) => Binding::Val(v.into()),
                            BindingImage::Proxy(i) => Binding::Proxy(
                                proxies.get(i as usize).cloned().flatten().ok_or_else(bad)?,
                            ),
                        };
                        es.push((k, b));
                    }
                    frames.push(Some(FrameRef::new(Frame {
                        entries: es,
                        parent,
                    })));
                    proxies.push(None);
                }
                NodeImage::Proxy { prog, frame } => {
                    let frame = match frame {
                        Some(i) => Some(frames.get(i as usize).cloned().flatten().ok_or_else(bad)?),
                        None => None,
                    };
                    proxies.push(Some(ProxyRef::new(Proxy { prog, frame })));
                    frames.push(None);
                }
            }
        }
        Ok(Rebuilt { frames })
    }

    fn frame(&self, i: u32) -> Result<FrameRef, String> {
        self.frames
            .get(i as usize)
            .cloned()
            .flatten()
            .ok_or_else(|| "damaged frame arena".to_string())
    }
}

/// A process.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProcImage {
    generation: u32,
    pid: u64,
    state: PState,
    kind: PKind,
    conts: Vec<Cont>,
    frames: Vec<u32>,
    results: Vec<u32>,
    acs: Vec<(AcRef, u32)>,
    elided: u32,
    in_think: bool,
    last_res: LastRes,
    table_seq: Option<u64>,
    fork_done: bool,
    rt: Vec<SeriesId>,
    ss_flags: u8,
    epoch: u32,
}

/// A recorded tape being replayed (part of the configuration).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TapeImage {
    uniforms: Vec<f64>,
    origins: Vec<String>,
    /// `s` records in key order: first uniform, uniforms used, value.
    samples: Vec<(u64, u64, ValueImage)>,
}

/// The run configuration without the run-time controls (deadline, cancellation flag).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ConfigImage {
    run_name: String,
    seed: i64,
    max_sim_time: i64,
    max_measurements: i64,
    simulate_linking_resources: bool,
    simulate_throughput_of_linking_resources: bool,
    /// `None`: the own stream; else the replayed tape.
    replay: Option<TapeImage>,
    check_tape_origins: bool,
    replay_samples: bool,
    store_measurements: bool,
    ps_algorithm: PsAlgorithm,
    max_events_per_instant: u64,
    max_stack_depth: u32,
    max_processes: u32,
    max_events: u64,
    max_steps: u64,
    mode: Mode,
    load_profile: Option<crate::load::LoadProfile>,
    usage_streams: bool,
}

impl From<&SimConfig> for ConfigImage {
    fn from(c: &SimConfig) -> Self {
        let replay = match &c.rng {
            RngMode::Own => None,
            RngMode::Replay(t) => {
                let mut samples: Vec<(u64, u64, ValueImage)> = t
                    .samples
                    .iter()
                    .map(|(k, (n, v))| (*k, *n, v.into()))
                    .collect();
                samples.sort_by_key(|s| s.0);
                Some(TapeImage {
                    uniforms: t.uniforms.clone(),
                    origins: t.origins.iter().map(|o| o.to_string()).collect(),
                    samples,
                })
            }
        };
        ConfigImage {
            run_name: c.run_name.clone(),
            seed: c.seed,
            max_sim_time: c.max_sim_time,
            max_measurements: c.max_measurements,
            simulate_linking_resources: c.simulate_linking_resources,
            simulate_throughput_of_linking_resources: c.simulate_throughput_of_linking_resources,
            replay,
            check_tape_origins: c.check_tape_origins,
            replay_samples: c.replay_samples,
            store_measurements: c.store_measurements,
            ps_algorithm: c.ps_algorithm,
            max_events_per_instant: c.max_events_per_instant,
            max_stack_depth: c.limits.max_stack_depth,
            max_processes: c.limits.max_processes,
            max_events: c.limits.max_events,
            max_steps: c.limits.max_steps,
            mode: c.mode,
            load_profile: c.load_profile.as_deref().cloned(),
            usage_streams: c.usage_streams,
        }
    }
}

impl From<ConfigImage> for SimConfig {
    fn from(c: ConfigImage) -> Self {
        let rng = match c.replay {
            None => RngMode::Own,
            Some(t) => RngMode::Replay(Arc::new(Tape {
                uniforms: t.uniforms,
                origins: t.origins.into_iter().map(Into::into).collect(),
                samples: t
                    .samples
                    .into_iter()
                    .map(|(k, n, v)| (k, (n, v.into())))
                    .collect(),
            })),
        };
        SimConfig {
            run_name: c.run_name,
            seed: c.seed,
            max_sim_time: c.max_sim_time,
            max_measurements: c.max_measurements,
            simulate_linking_resources: c.simulate_linking_resources,
            simulate_throughput_of_linking_resources: c.simulate_throughput_of_linking_resources,
            rng,
            check_tape_origins: c.check_tape_origins,
            replay_samples: c.replay_samples,
            store_measurements: c.store_measurements,
            ps_algorithm: c.ps_algorithm,
            max_events_per_instant: c.max_events_per_instant,
            limits: Limits {
                max_stack_depth: c.max_stack_depth,
                max_processes: c.max_processes,
                max_events: c.max_events,
                max_steps: c.max_steps,
                deadline: None,
                cancel: None,
            },
            mode: c.mode,
            load_profile: c.load_profile.map(Arc::new),
            usage_streams: c.usage_streams,
        }
    }
}

/// The section `engine`: everything of [`RunState`] except the run's own model (section
/// `model`) and what is zero or empty between events (nesting counters, allocation pools).
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
struct EngineImage<K: Clock> {
    cfg: ConfigImage,
    phase: Phase,
    failure: Option<SimError>,
    traced: bool,
    now: i64,
    running: bool,
    n_events: u64,
    same_instant: u64,
    count_seen: i64,
    main_count: i64,
    frames: Vec<NodeImage>,
    empty: u32,
    procs: Vec<ProcImage>,
    free: Vec<u32>,
    table_seq: u64,
    joins: Vec<Vec<u64>>,
    free_joins: Vec<u32>,
    res: Vec<ResRt<K>>,
    passives: Vec<PassiveInst>,
    comps: Vec<Option<CompInst>>,
    dyn_paths: Vec<AcPath>,
    dyn_path_child: Vec<((u32, AcRef), u32)>,
    rng: RngImage,
    math: crate::javarand::JavaRandom,
    ss_time_stop: bool,
    meas: Sink,
    #[serde(with = "crate::snapshot::serde_util::series_map")]
    run_series: HashMap<Box<str>, Vec<(&'static str, SeriesId)>>,
    warnings: Vec<String>,
    /// The run model's generation (0: the base model, no section `model`).
    generation: u32,
    changes: crate::change::ChangeOptions,
    provs: Vec<ProvRes>,
    prov_ids: Vec<((AcRef, RoleId, SignatureId), u32)>,
    call_cache: Vec<CallCache>,
    elsc_prov: Vec<u32>,
    steps: u64,
    next_step_check: u64,
    next_event_check: u64,
    /// The load profile's run state (`crate::load`).
    lp: Option<crate::load::LoadRun>,
    /// The usage streams (`super::streams`).
    us: Option<UsageStreamsImage>,
}

/// The usage streams of a run (`super::streams::UsageStreams`).
#[derive(Serialize, Deserialize)]
struct UsageStreamsImage {
    /// Per scenario: seed, arrival stream, users created.
    scen: Vec<Option<(u64, RngImage, u64)>>,
    /// Per process slot: the user's stream.
    user: Vec<Option<RngImage>>,
}

/// The section `model` as written (borrowing the compiled model).
#[derive(Serialize)]
struct ModelOut<'a> {
    generation: u32,
    #[serde(with = "crate::snapshot::serde_util::objids")]
    order: &'a [ObjId],
    graph: GraphXmi,
    compiled: &'a CompiledModel,
}

/// The section `model` as read.
#[derive(Deserialize)]
struct ModelIn {
    generation: u32,
    #[serde(with = "crate::snapshot::serde_util::objids")]
    order: Vec<ObjId>,
    graph: GraphXmi,
    compiled: CompiledModel,
}

fn sorted<K: Ord + Clone + std::hash::Hash, V: Clone>(m: &FxMap<K, V>) -> Vec<(K, V)> {
    let mut v: Vec<(K, V)> = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// The section `model` of a run's own compiled model (generation `generation`, typed model
/// built in `order`), and its line of the header.
pub(crate) fn model_section(
    cm: &CompiledModel,
    order: &[ObjId],
    generation: u32,
    enc: Encoding,
) -> Result<(Section, RunModelInfo), SnapshotError> {
    let m = ModelOut {
        generation,
        order,
        graph: GraphXmi::save(&cm.model.graph),
        compiled: cm,
    };
    Ok((
        Section::encode(MODEL, &m, enc)?,
        RunModelInfo {
            generation,
            fingerprint: model_fingerprint(cm),
        },
    ))
}

/// The run's own compiled model of snapshot `s` (section `model`, generation `generation`)
/// and its build order, checked against the header; for a run created on `base`.
pub(crate) fn read_model_section(
    s: &Snapshot,
    base: &CompiledModel,
    generation: u32,
) -> Result<(CompiledModel, Vec<ObjId>), SnapshotError> {
    let restore = |m: String| SnapshotError::Restore(m);
    let sec = s
        .section(MODEL)
        .ok_or_else(|| SnapshotError::Corrupt("no section `model` after model changes".into()))?;
    let m: ModelIn = sec.decode(s.header.encoding)?;
    if m.generation != generation {
        return Err(SnapshotError::Corrupt("model generation mismatch".into()));
    }
    let graph = m.graph.restore().map_err(restore)?;
    let (typed, order) = simoxide_model::build_from_graph_after(
        graph,
        simoxide_model::Diagnostics::default(),
        &m.order,
    );
    if order != m.order {
        return Err(SnapshotError::Corrupt(
            "the build order does not match the snapshot's model graph".into(),
        ));
    }
    let mut cm = m.compiled;
    crate::snapshot::restore_compiled(&mut cm, typed).map_err(restore)?;
    if cm.flavour != base.flavour {
        return Err(SnapshotError::Corrupt(
            "the run's model has another flavour".into(),
        ));
    }
    let fp = model_fingerprint(&cm);
    match &s.header.run_model {
        Some(rm) if rm.fingerprint == fp && rm.generation == m.generation => {}
        _ => {
            return Err(SnapshotError::Restore(format!(
                "the restored run model differs from the one saved (fingerprint {fp})"
            )));
        }
    }
    Ok((cm, order))
}

/// The section `graph`: a run's model graph that differs from its compiled model's outside
/// SimOxide's metamodels.
pub(crate) fn graph_section(
    g: &simoxide_model::raw::Graph,
    enc: Encoding,
) -> Result<Section, SnapshotError> {
    Section::encode(GRAPH, &GraphXmi::save(g), enc)
}

/// The graph of the section `graph` of `s`, if it has one.
pub(crate) fn read_graph_section(
    s: &Snapshot,
) -> Result<Option<simoxide_model::raw::Graph>, SnapshotError> {
    let Some(sec) = s.section(GRAPH) else {
        return Ok(None);
    };
    let x: GraphXmi = sec.decode(s.header.encoding)?;
    x.restore().map(Some).map_err(SnapshotError::Restore)
}

impl<C: Compat, F: Flavour> RunState<C, F> {
    fn engine_image(&self) -> Result<EngineImage<F::Clock>, SnapshotError> {
        if self.nesting != 0 || self.direct != 0 {
            return Err(SnapshotError::Encode(
                "the run is inside an event (snapshots are taken between events)".into(),
            ));
        }
        let mut arena = Arena::default();
        let empty = arena.frame(&self.empty);
        let procs = self
            .procs
            .iter()
            .map(|p| {
                let d = &p.data;
                ProcImage {
                    generation: p.generation,
                    pid: p.pid,
                    state: p.state,
                    kind: p.kind,
                    conts: d.conts.clone(),
                    frames: d.frames.iter().map(|f| arena.frame(f)).collect(),
                    results: d.results.iter().map(|f| arena.frame(f)).collect(),
                    acs: d.acs.clone(),
                    elided: d.elided,
                    in_think: p.in_think,
                    last_res: p.last_res,
                    table_seq: p.table_seq,
                    fork_done: p.fork_done,
                    rt: p.rt.clone(),
                    ss_flags: p.ss_flags,
                    epoch: p.epoch,
                }
            })
            .collect();
        Ok(EngineImage {
            cfg: (&self.cfg).into(),
            phase: self.phase,
            failure: self.failure.clone(),
            traced: self.traced,
            now: self.now,
            running: self.running,
            n_events: self.n_events,
            same_instant: self.same_instant,
            count_seen: self.count_seen,
            main_count: self.main_count,
            frames: arena.nodes,
            empty,
            procs,
            free: self.free.clone(),
            table_seq: self.table_seq,
            joins: self.joins.clone(),
            free_joins: self.free_joins.clone(),
            res: self.res.clone(),
            passives: self.passives.clone(),
            comps: self.comps.clone(),
            dyn_paths: self.dyn_paths.clone(),
            dyn_path_child: sorted(&self.dyn_path_child),
            rng: self.rng.save(),
            math: self.math.clone(),
            ss_time_stop: self.ss_time_stop,
            meas: self.meas.clone(),
            run_series: self.run_series.clone(),
            warnings: self.warnings.clone(),
            generation: self.model.generation(),
            changes: self.changes,
            provs: self.provs.clone(),
            prov_ids: sorted(&self.prov_ids),
            call_cache: self.call_cache.clone(),
            elsc_prov: self.elsc_prov.clone(),
            steps: self.steps,
            next_step_check: self.next_step_check,
            next_event_check: self.next_event_check,
            lp: self.lp.as_deref().cloned(),
            us: self.us.as_deref().map(|u| UsageStreamsImage {
                scen: u
                    .scen
                    .iter()
                    .map(|x| x.as_ref().map(|x| (x.seed, x.arrivals.save(), x.users)))
                    .collect(),
                user: u
                    .user
                    .iter()
                    .map(|x| x.as_ref().map(SimRng::save))
                    .collect(),
            }),
        })
    }

    /// The snapshot of this state (between events; the run was created on `base`), with the
    /// sections `engine` and, after model changes, `model`, encoded with `enc`.
    ///
    /// The state of a running [`Simulation`] is best written with [`Simulation::snapshot_to`],
    /// which flushes the outputs first.
    pub fn serialize(
        &self,
        base: &CompiledModel,
        enc: Encoding,
    ) -> Result<Snapshot, SnapshotError> {
        let mut sections = vec![Section::encode(ENGINE, &self.engine_image()?, enc)?];
        let mut run_model = None;
        if let (Some(cm), Some(order)) = (self.model.compiled(), self.model.order()) {
            let (sec, info) = model_section(cm, order, self.model.generation(), enc)?;
            sections.push(sec);
            run_model = Some(info);
        }
        if let Some(g) = self.model.graph_override() {
            sections.push(graph_section(g, enc)?);
        }
        let header = Header {
            format: FORMAT,
            simoxide: env!("CARGO_PKG_VERSION").to_string(),
            mode: C::MODE.name().to_string(),
            literal: C::LITERAL_ENGINE,
            flavour: F::KIND.name().to_string(),
            quirks: if base.flavour.quirks.is_empty() {
                String::new()
            } else {
                base.flavour.quirks.to_string()
            },
            model: model_fingerprint(base),
            run_model,
            time_ns: F::Clock::to_ns(self.now),
            events: self.n_events,
            phase: format!("{:?}", self.phase).to_lowercase(),
            traced: self.traced,
            encoding: enc,
            sections: Vec::new(),
        };
        Ok(Snapshot { header, sections })
    }

    /// The state of snapshot `s`, for a run created on `base` (which must be the compiled model
    /// the snapshot names; after model changes the state brings its own model).
    pub fn deserialize(
        s: &Snapshot,
        base: &CompiledModel,
    ) -> Result<RunState<C, F>, SnapshotError> {
        let h = &s.header;
        let this = env!("CARGO_PKG_VERSION");
        if h.simoxide != this {
            return Err(SnapshotError::SimOxideVersion {
                found: h.simoxide.clone(),
                this: this.to_string(),
            });
        }
        if h.mode != C::MODE.name() || h.literal != C::LITERAL_ENGINE {
            return Err(SnapshotError::Mismatch(format!(
                "the snapshot is of a {}{} mode run; this simulation runs the {}{} mode",
                h.mode,
                if h.literal { " (literal engine)" } else { "" },
                C::MODE.name(),
                if C::LITERAL_ENGINE {
                    " (literal engine)"
                } else {
                    ""
                },
            )));
        }
        if h.flavour != F::KIND.name() || base.flavour.kind != F::KIND {
            return Err(SnapshotError::Mismatch(format!(
                "the snapshot is of a {} flavour run; this simulation runs the {} flavour on a model compiled for {}",
                h.flavour,
                F::KIND.name(),
                base.flavour.kind.name()
            )));
        }
        let fp = model_fingerprint(base);
        if fp != h.model {
            return Err(SnapshotError::Model {
                expected: h.model.clone(),
                found: fp,
            });
        }
        let enc = h.encoding;
        let engine = s
            .section(ENGINE)
            .ok_or_else(|| SnapshotError::Corrupt("no section `engine`".into()))?;
        let img: EngineImage<F::Clock> = engine.decode(enc)?;
        let restore = |m: String| SnapshotError::Restore(m);
        // the run's own model
        let model = if img.generation == 0 {
            RunModel::default()
        } else {
            let (cm, order) = read_model_section(s, base, img.generation)?;
            RunModel::changed(Arc::new(cm), order.into(), Shape::of(base), img.generation)
        };
        let mut model = model;
        if let Some(g) = read_graph_section(s)? {
            model.set_graph_override(Arc::new(g));
        }
        let cm: &CompiledModel = match model.compiled() {
            Some(a) => a,
            None => base,
        };
        // plausibility of the tables the state indexes into
        if img.res.len() != cm.resources.len()
            || img.call_cache.len() != cm.code.traced.call_sites as usize
            || img.elsc_prov.len() != cm.uact.len()
            || img.meas.m.series.len() < cm.series.len()
        {
            return Err(SnapshotError::Corrupt(
                "the run state does not fit the model's tables".into(),
            ));
        }
        let cfg: SimConfig = img.cfg.into();
        let rng = C::Rng::load(img.rng, &cfg).map_err(restore)?;
        let us = match img.us {
            None => None,
            Some(u) => {
                let mut scen = Vec::with_capacity(u.scen.len());
                for x in u.scen {
                    scen.push(match x {
                        None => None,
                        Some((seed, r, users)) => Some(super::streams::ScenStreams {
                            seed,
                            arrivals: C::Rng::load(r, &cfg).map_err(restore)?,
                            users,
                        }),
                    });
                }
                let mut user = Vec::with_capacity(u.user.len());
                for x in u.user {
                    user.push(match x {
                        None => None,
                        Some(r) => Some(C::Rng::load(r, &cfg).map_err(restore)?),
                    });
                }
                Some(Box::new(super::streams::UsageStreams { scen, user }))
            }
        };
        let frames = Rebuilt::new(img.frames).map_err(SnapshotError::Corrupt)?;
        let empty = frames.frame(img.empty).map_err(SnapshotError::Corrupt)?;
        let mut procs = Vec::with_capacity(img.procs.len());
        for p in img.procs {
            let fs = |v: Vec<u32>| -> Result<Vec<FrameRef>, SnapshotError> {
                v.into_iter()
                    .map(|i| frames.frame(i).map_err(SnapshotError::Corrupt))
                    .collect()
            };
            procs.push(Proc {
                generation: p.generation,
                pid: p.pid,
                state: p.state,
                kind: p.kind,
                data: Box::new(ProcData {
                    conts: p.conts,
                    frames: fs(p.frames)?,
                    results: fs(p.results)?,
                    acs: p.acs,
                    elided: p.elided,
                }),
                in_think: p.in_think,
                last_res: p.last_res,
                table_seq: p.table_seq,
                fork_done: p.fork_done,
                rt: p.rt,
                ss_flags: p.ss_flags,
                epoch: p.epoch,
            });
        }
        let max_depth = max_depth(&cfg.limits);
        Ok(RunState {
            phase: img.phase,
            failure: img.failure,
            traced: img.traced,
            shape: Shape::of(cm),
            now: img.now,
            running: img.running,
            n_events: img.n_events,
            same_instant: img.same_instant,
            count_seen: img.count_seen,
            main_count: img.main_count,
            procs,
            free: img.free,
            table_seq: img.table_seq,
            empty,
            joins: img.joins,
            free_joins: img.free_joins,
            res: img.res,
            passives: img.passives,
            comps: img.comps,
            dyn_paths: img.dyn_paths,
            dyn_path_child: img.dyn_path_child.into_iter().collect(),
            rng,
            math: img.math,
            ss_time_stop: img.ss_time_stop,
            meas: img.meas,
            run_series: img.run_series,
            warnings: img.warnings,
            model,
            changes: img.changes,
            provs: img.provs,
            prov_ids: img.prov_ids.into_iter().collect(),
            call_cache: img.call_cache,
            elsc_prov: img.elsc_prov,
            steps: img.steps,
            next_step_check: img.next_step_check,
            next_event_check: img.next_event_check,
            lp: img.lp.map(Box::new),
            us,
            max_depth,
            nesting: 0,
            direct: 0,
            fork_scratch: Vec::new(),
            pool: FramePool::default(),
            spare_data: Vec::new(),
            cfg,
        })
    }
}

impl<'m, C: Compat, F: Flavour> Simulation<'m, C, F> {
    /// The snapshot of the run, between events (outputs flushed first, as [`Simulation::snapshot`]
    /// does): the sections `engine` (and `model` after model changes) plus `extra` (sections of
    /// layers above the engine, encoded with the same encoding).
    pub fn snapshot_value(
        &mut self,
        enc: Encoding,
        extra: Vec<Section>,
    ) -> Result<Snapshot, SnapshotError> {
        self.flush_outputs();
        let mut s = self.st.serialize(self.base, enc)?;
        s.sections.extend(extra);
        Ok(s)
    }

    /// Writes a snapshot of the run (between events) to `w` in the default encoding (postcard).
    /// [`Simulation::restore_from`] continues it exactly like the uninterrupted run.
    pub fn snapshot_to(&mut self, w: impl std::io::Write) -> Result<SnapshotInfo, SnapshotError> {
        self.snapshot_with(w, &SnapshotOptions::default(), Vec::new())
    }

    /// [`Simulation::snapshot_to`] with options and extra sections.
    pub fn snapshot_with(
        &mut self,
        w: impl std::io::Write,
        opts: &SnapshotOptions,
        extra: Vec<Section>,
    ) -> Result<SnapshotInfo, SnapshotError> {
        let s = self.snapshot_value(opts.encoding, extra)?;
        let mut counter = Counting { inner: w, n: 0 };
        let header = s.write(&mut counter)?;
        Ok(SnapshotInfo {
            header,
            bytes: counter.n,
        })
    }

    /// Restores a snapshot written by [`Simulation::snapshot_to`]: the run continues on `base`
    /// (the compiled model it was created on; checked by fingerprint) with the outputs `out`,
    /// which receive the continuation only (see [`Simulation::resume`]).
    pub fn restore_from(
        r: impl std::io::Read,
        base: &'m CompiledModel,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        let s = Snapshot::read(r)?;
        Self::restore_snapshot(&s, base, out)
    }

    /// [`Simulation::restore_from`] of a snapshot already read (layers above the engine read
    /// their own sections from it).
    pub fn restore_snapshot(
        s: &Snapshot,
        base: &'m CompiledModel,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        let st = RunState::<C, F>::deserialize(s, base)?;
        Simulation::resume(base, st, out).map_err(|e| SnapshotError::Restore(e.message))
    }
}

/// Counts the bytes written.
struct Counting<W> {
    inner: W,
    n: u64,
}

impl<W: std::io::Write> std::io::Write for Counting<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
