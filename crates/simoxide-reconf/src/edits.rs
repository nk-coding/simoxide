//! Change files (`DESIGN-SPD.md` N2): the common MENTOR parameter changes as text, one change
//! per line, applied to a run (e.g. a restored snapshot) as one change set through [`Edit`] and
//! [`Simulation::apply_changes`]. The format is documented in `docs/guide/snapshots.md`
//! ("Change files").
//!
//! ```text
//! # processing rate of a container's CPU, a demand, a branch probability, the workload
//! rate         Server1:CPU   2000
//! demand       _internalAction1:CPU   12 * file.BYTESIZE
//! probability  _transition1  0.3
//! think-time   "Default Scenario"  Exp(0.5)
//! arrival-rate _openScenario  4.0
//! # a new container like Server1 (on the same links), and an allocation onto it
//! add-container Server1 _server3 "Server 3"
//! allocate     _assemblyCtx2 _server3 _alloc3
//! ```
//!
//! * A line is `OP ARGUMENTS`; `#` starts a comment line. Arguments are separated by blanks; a
//!   name with blanks is written in double quotes. The value of a value change (a StoEx, a
//!   number) is the rest of the line, as written.
//! * An element (`REF`) is named by its id (`Identifier.id`) or, if no element has that id, by
//!   its name (`entityName`, unique among the elements of the kind the change expects, e.g.
//!   assembly contexts for `allocate`). `REF:TYPE` selects a container's processing
//!   resource, or an internal action's demand, by resource type (`CPU`, `HDD`, `DELAY`, or the
//!   type's id).
//! * Lines run in order on one copy of the run's model (an element created by a line can be
//!   named by later ones) and are applied together; on any error nothing changes.

use crate::{Edit, EditError};
use qvto_model::{Datum, ModelStore};
use simoxide_model::meta::prelude::*;
use simoxide_model::meta::{ClassId, DataKind, FeatureId, class as mc, feat as mf};
use simoxide_model::raw::{Graph, ObjId, PcmGraph, Value};
use simoxide_sim::{ChangeError, ChangeReport, Compat, Flavour, Simulation};
use std::fmt;
use std::path::Path;

/// One change of a [`ChangeFile`].
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp {
    /// `rate REF[:TYPE] STOEX`: processing rate of a processing resource (a container's, by type).
    Rate {
        target: Target,
        spec: String,
    },
    /// `latency LINK STOEX`, `throughput LINK STOEX`: of a linking resource.
    Latency {
        link: String,
        spec: String,
    },
    Throughput {
        link: String,
        spec: String,
    },
    /// `demand ACTION[:TYPE] STOEX`: a parametric resource demand of an internal action.
    Demand {
        target: Target,
        spec: String,
    },
    /// `probability TRANSITION P`: a branch transition's probability (SEFF or usage model).
    Probability {
        transition: String,
        p: f64,
    },
    /// `think-time WORKLOAD STOEX`: of a closed workload (or a usage scenario's).
    ThinkTime {
        workload: String,
        spec: String,
    },
    /// `inter-arrival WORKLOAD STOEX`: of an open workload (or a usage scenario's).
    InterArrival {
        workload: String,
        spec: String,
    },
    /// `arrival-rate WORKLOAD RATE`: inter-arrival time `Exp(RATE)`.
    ArrivalRate {
        workload: String,
        rate: f64,
    },
    /// `population WORKLOAD N`: of a closed workload (not supported by the change path yet: F4).
    Population {
        workload: String,
        n: i32,
    },
    /// `replicas REF[:TYPE] N`: number of replicas (cores) of a processing resource (SimuLizar's
    /// syncers ignore it: MC-1.4).
    Replicas {
        target: Target,
        n: i32,
    },
    /// `loop-count LOOP STOEX`: iteration count of a SEFF loop or a usage-model loop.
    LoopCount {
        action: String,
        spec: String,
    },
    /// `set REF FEATURE VALUE`: any single-valued attribute (by its EMF name), a random variable
    /// (its specification), or a reference (VALUE is a REF).
    Set {
        target: String,
        feature: String,
        value: String,
    },
    /// `add-container TEMPLATE NEW_ID [NAME]`: a container with copies of the template's
    /// processing resources, connected to the template's linking resources.
    AddContainer {
        template: String,
        id: String,
        name: Option<String>,
    },
    /// `connect LINK CONTAINER`.
    Connect {
        link: String,
        container: String,
    },
    /// `add-assembly TEMPLATE NEW_ID [NAME]`: an assembly context of the same component.
    AddAssembly {
        template: String,
        id: String,
        name: Option<String>,
    },
    /// `allocate ASSEMBLY CONTAINER NEW_ID`: a new allocation context.
    Allocate {
        assembly: String,
        container: String,
        id: String,
    },
    /// `remove-container CONTAINER`: out of the resource environment (it keeps simulating, MC-1.5).
    RemoveContainer {
        container: String,
    },
    /// `delete REF`: `EcoreUtil.delete`.
    Delete {
        target: String,
    },
    /// `clone-monitors TEMPLATE CONTAINER`: copies of the template container's monitors for the
    /// container (Slingshot's `ResourceContainerMonitorCloner`, [`Edit::clone_monitors`]; ids
    /// `<container id>_mon_<n>`).
    CloneMonitors {
        template: String,
        container: String,
    },
    /// `add-demand ACTION:TYPE STOEX`: a new parametric resource demand of an internal action
    /// (a structural change of the action).
    AddDemand {
        target: Target,
        spec: String,
    },
    /// `removal keep|drain|abort|slingshot-q5`, `in-progress follow|finish`,
    /// `reallocation ignore|apply`, `monitors reference|full`: the run's change options
    /// ([`simoxide_sim::ChangeOptions`]) from this change set on (they stay with the run).
    Options(OptionOp),
}

/// A change-option line of a [`ChangeFile`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OptionOp {
    Removal(simoxide_sim::Removal),
    InProgress(simoxide_sim::InProgress),
    Reallocation(simoxide_sim::Reallocation),
    Monitors(simoxide_sim::MonitorSync),
}

impl OptionOp {
    fn parse(op: &str, v: &str) -> Option<OptionOp> {
        use simoxide_sim::{InProgress, MonitorSync, Reallocation, Removal};
        Some(match (op, v) {
            ("removal", "keep") => OptionOp::Removal(Removal::Keep),
            ("removal", "drain") => OptionOp::Removal(Removal::Drain),
            ("removal", "abort") => OptionOp::Removal(Removal::Abort),
            ("removal", "slingshot-q5") => OptionOp::Removal(Removal::SlingshotQ5),
            ("in-progress", "follow") => OptionOp::InProgress(InProgress::Follow),
            ("in-progress", "finish") => OptionOp::InProgress(InProgress::Finish),
            ("reallocation", "ignore") => OptionOp::Reallocation(Reallocation::Ignore),
            ("reallocation", "apply") => OptionOp::Reallocation(Reallocation::Apply),
            ("monitors", "reference") => OptionOp::Monitors(MonitorSync::Reference),
            ("monitors", "full") => OptionOp::Monitors(MonitorSync::Full),
            _ => return None,
        })
    }

    fn set(self, o: &mut simoxide_sim::ChangeOptions) {
        match self {
            OptionOp::Removal(x) => o.removal = x,
            OptionOp::InProgress(x) => o.in_progress = x,
            OptionOp::Reallocation(x) => o.reallocation = x,
            OptionOp::Monitors(x) => o.monitors = x,
        }
    }
}

/// `REF` or `REF:TYPE`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub element: String,
    pub resource_type: Option<String>,
}

/// A parsed change file: its changes with their line numbers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeFile {
    pub ops: Vec<(usize, EditOp)>,
}

/// An error of a change file: a line that does not parse or cannot be applied, or the change
/// path's refusal of the whole set.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeFileError {
    /// Line (1-based) and message.
    Line(usize, String),
    /// The changes are valid edits but the run cannot take them ([`ChangeError`]: e.g.
    /// `Unsupported` for population changes or a Slingshot-flavour run, `Abort` for a removed
    /// allocation context). The run is unchanged.
    Change(ChangeError),
    Io(String),
}

impl fmt::Display for ChangeFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeFileError::Line(l, m) => write!(f, "line {l}: {m}"),
            ChangeFileError::Change(e) => e.fmt(f),
            ChangeFileError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ChangeFileError {}

/// Splits a line into tokens (blank-separated, `"..."` for blanks); also returns where each
/// token ends, for "the rest of the line".
fn tokens(line: &str) -> Result<Vec<(String, usize)>, String> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i] == b'"' {
            let end = line[i + 1..]
                .find('"')
                .ok_or("unterminated quotes")?
                .saturating_add(i + 1);
            out.push((line[i + 1..end].to_string(), end + 1));
            i = end + 1;
        } else {
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push((line[start..i].to_string(), i));
        }
    }
    Ok(out)
}

impl ChangeFile {
    /// Parses a change file.
    pub fn parse(text: &str) -> Result<ChangeFile, ChangeFileError> {
        let mut ops = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line_no = n + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let e = |m: String| ChangeFileError::Line(line_no, m);
            let t = tokens(line).map_err(|m| e(m.to_string()))?;
            let op = t[0].0.as_str();
            // the rest of the line after `k` tokens (the value), required
            let rest = |k: usize, what: &str| -> Result<String, ChangeFileError> {
                let r = t.get(k - 1).map(|x| line[x.1..].trim()).unwrap_or_default();
                if r.is_empty() || t.len() <= k {
                    return Err(e(format!("{op}: {what} missing")));
                }
                Ok(r.to_string())
            };
            let arg = |k: usize, what: &str| -> Result<String, ChangeFileError> {
                t.get(k)
                    .map(|x| x.0.clone())
                    .ok_or_else(|| e(format!("{op}: {what} missing")))
            };
            let at_most = |k: usize| -> Result<(), ChangeFileError> {
                if t.len() > k {
                    return Err(e(format!("{op}: unexpected `{}`", t[k].0)));
                }
                Ok(())
            };
            let target = |s: String| -> Target {
                match s.rsplit_once(':') {
                    Some((el, ty)) if !el.is_empty() && !ty.is_empty() => Target {
                        element: el.to_string(),
                        resource_type: Some(ty.to_string()),
                    },
                    _ => Target {
                        element: s,
                        resource_type: None,
                    },
                }
            };
            let number = |s: String| -> Result<f64, ChangeFileError> {
                s.parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| e(format!("{op}: not a number: {s}")))
            };
            let int = |s: String| -> Result<i32, ChangeFileError> {
                s.parse::<i32>()
                    .map_err(|_| e(format!("{op}: not an integer: {s}")))
            };
            let o = match op {
                "rate" => EditOp::Rate {
                    target: target(arg(1, "resource")?),
                    spec: rest(2, "rate")?,
                },
                "latency" => EditOp::Latency {
                    link: arg(1, "linking resource")?,
                    spec: rest(2, "latency")?,
                },
                "throughput" => EditOp::Throughput {
                    link: arg(1, "linking resource")?,
                    spec: rest(2, "throughput")?,
                },
                "demand" => EditOp::Demand {
                    target: target(arg(1, "internal action")?),
                    spec: rest(2, "demand")?,
                },
                "probability" => EditOp::Probability {
                    transition: arg(1, "branch transition")?,
                    p: number(rest(2, "probability")?)?,
                },
                "think-time" => EditOp::ThinkTime {
                    workload: arg(1, "workload")?,
                    spec: rest(2, "think time")?,
                },
                "inter-arrival" => EditOp::InterArrival {
                    workload: arg(1, "workload")?,
                    spec: rest(2, "inter-arrival time")?,
                },
                "arrival-rate" => {
                    let rate = number(rest(2, "rate")?)?;
                    if rate <= 0.0 {
                        return Err(e("arrival-rate: expected a rate > 0".into()));
                    }
                    EditOp::ArrivalRate {
                        workload: arg(1, "workload")?,
                        rate,
                    }
                }
                "population" => EditOp::Population {
                    workload: arg(1, "workload")?,
                    n: int(rest(2, "population")?)?,
                },
                "replicas" => EditOp::Replicas {
                    target: target(arg(1, "resource")?),
                    n: int(rest(2, "replicas")?)?,
                },
                "loop-count" => EditOp::LoopCount {
                    action: arg(1, "loop")?,
                    spec: rest(2, "iteration count")?,
                },
                "set" => EditOp::Set {
                    target: arg(1, "element")?,
                    feature: arg(2, "feature")?,
                    value: rest(3, "value")?,
                },
                "add-container" => {
                    at_most(4)?;
                    EditOp::AddContainer {
                        template: arg(1, "template container")?,
                        id: arg(2, "new id")?,
                        name: t.get(3).map(|x| x.0.clone()),
                    }
                }
                "connect" => {
                    at_most(3)?;
                    EditOp::Connect {
                        link: arg(1, "linking resource")?,
                        container: arg(2, "container")?,
                    }
                }
                "add-assembly" => {
                    at_most(4)?;
                    EditOp::AddAssembly {
                        template: arg(1, "template assembly context")?,
                        id: arg(2, "new id")?,
                        name: t.get(3).map(|x| x.0.clone()),
                    }
                }
                "allocate" => {
                    at_most(4)?;
                    EditOp::Allocate {
                        assembly: arg(1, "assembly context")?,
                        container: arg(2, "container")?,
                        id: arg(3, "new id")?,
                    }
                }
                "remove-container" => {
                    at_most(2)?;
                    EditOp::RemoveContainer {
                        container: arg(1, "container")?,
                    }
                }
                "delete" => {
                    at_most(2)?;
                    EditOp::Delete {
                        target: arg(1, "element")?,
                    }
                }
                "clone-monitors" => {
                    at_most(3)?;
                    EditOp::CloneMonitors {
                        template: arg(1, "template container")?,
                        container: arg(2, "container")?,
                    }
                }
                "add-demand" => EditOp::AddDemand {
                    target: target(arg(1, "internal action:resource type")?),
                    spec: rest(2, "demand")?,
                },
                "removal" | "in-progress" | "reallocation" | "monitors" => {
                    at_most(2)?;
                    let v = arg(1, "value")?;
                    EditOp::Options(
                        OptionOp::parse(op, &v)
                            .ok_or_else(|| e(format!("{op}: unknown value `{v}`")))?,
                    )
                }
                o => return Err(e(format!("unknown change `{o}`"))),
            };
            ops.push((line_no, o));
        }
        Ok(ChangeFile { ops })
    }

    /// Reads and parses a change file.
    pub fn load(path: &Path) -> Result<ChangeFile, ChangeFileError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ChangeFileError::Io(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    /// Makes the changes on a copy of `sim`'s model and applies them as one change set
    /// ([`Simulation::apply_changes`]). On an error the run is unchanged.
    pub fn apply<C: Compat, F: Flavour>(
        &self,
        sim: &mut Simulation<'_, C, F>,
    ) -> Result<ChangeReport, ChangeFileError> {
        let mut e = Edit::new(sim);
        let types: Vec<(ObjId, Box<str>, Box<str>)> = sim
            .compiled()
            .model
            .resource_types
            .iter()
            .map(|t| (t.obj, t.id.clone(), t.name.clone()))
            .collect();
        let before = sim.change_options();
        let mut opts = before;
        for (line, op) in &self.ops {
            if let EditOp::Options(o) = op {
                o.set(&mut opts);
                continue;
            }
            let mut cx = Cx {
                e: &mut e,
                types: &types,
            };
            cx.op(op)
                .map_err(|m| ChangeFileError::Line(*line, m.to_string()))?;
        }
        sim.set_change_options(opts);
        let r = e.apply(sim).map_err(ChangeFileError::Change);
        if r.is_err() {
            sim.set_change_options(before);
        }
        r
    }
}

impl ChangeFile {
    /// [`ChangeFile::apply`] on any run that takes model changes (`ChangeHost`, e.g. a paused
    /// Slingshot-exact run, `simoxide_sim::ssx::SsxRun`). Such hosts have no change options:
    /// a file with option lines (`removal`, `in-progress`, `reallocation`, `monitors`) is
    /// refused.
    pub fn apply_to<H: simoxide_sim::ChangeHost + ?Sized>(
        &self,
        host: &mut H,
    ) -> Result<ChangeReport, ChangeFileError> {
        let mut e = Edit::new(host);
        let types: Vec<(ObjId, Box<str>, Box<str>)> = host
            .compiled_now()
            .model
            .resource_types
            .iter()
            .map(|t| (t.obj, t.id.clone(), t.name.clone()))
            .collect();
        for (line, op) in &self.ops {
            if let EditOp::Options(_) = op {
                return Err(ChangeFileError::Line(
                    *line,
                    "change options are not supported by this run (they set the SimuLizar and \
                     Slingshot flavours' change machinery)"
                        .into(),
                ));
            }
            let mut cx = Cx {
                e: &mut e,
                types: &types,
            };
            cx.op(op)
                .map_err(|m| ChangeFileError::Line(*line, m.to_string()))?;
        }
        e.apply(host).map_err(ChangeFileError::Change)
    }
}

/// Applies the ops of a file on an [`Edit`].
struct Cx<'a> {
    e: &'a mut Edit,
    /// Resource types of the run's model: object, id, name.
    types: &'a [(ObjId, Box<str>, Box<str>)],
}

type R<T> = Result<T, EditError>;

fn err<T>(m: impl Into<String>) -> R<T> {
    Err(EditError(m.into()))
}

impl Cx<'_> {
    fn g(&self) -> &Graph {
        self.e.graph()
    }

    /// The element named by `r`: its id, else its unique name.
    fn find(&self, r: &str) -> R<ObjId> {
        self.find_as(r, &[])
    }

    /// The element named by `r`: its id, else its name, unique among the elements of the
    /// classes `cs` (any class if empty).
    fn find_as(&self, r: &str, cs: &[ClassId]) -> R<ObjId> {
        if let Some(o) = self.e.find(r) {
            return Ok(o);
        }
        let g = self.g();
        let named: Vec<ObjId> = (0..g.resources().len() as u32)
            .flat_map(|res| g.tree_contents(simoxide_model::raw::ResId(res)))
            .filter(|&o| g.str_attr(o, mf::NamedElement_entityName).as_deref() == Some(r))
            .collect();
        let typed: Vec<ObjId> = named
            .iter()
            .copied()
            .filter(|&o| cs.is_empty() || cs.iter().any(|&c| g[o].class.is_a(c)))
            .collect();
        let named = if typed.is_empty() { named } else { typed };
        match named.as_slice() {
            [o] => Ok(*o),
            [] => err(format!("no element with id or name `{r}`")),
            many => err(format!(
                "the name `{r}` is ambiguous ({} elements: {}); use an id",
                many.len(),
                many.iter()
                    .filter_map(|&o| g.id(o))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn is(&self, o: ObjId, c: ClassId) -> bool {
        self.g()[o].class.is_a(c)
    }

    fn expect(&self, o: ObjId, c: ClassId, what: &str) -> R<()> {
        if self.is(o, c) {
            Ok(())
        } else {
            err(format!(
                "{} is a {}, not a {what}",
                self.g().describe(o),
                self.g()[o].class.name()
            ))
        }
    }

    /// Whether the resource type object `t` (a proxy or loaded) is `name` (name or id).
    fn type_is(&self, t: Option<ObjId>, name: &str) -> bool {
        let Some(t) = t else { return false };
        if let Some((_, id, n)) = self.types.iter().find(|x| x.0 == t) {
            return &**id == name || n.eq_ignore_ascii_case(name);
        }
        let g = self.g();
        g.id(t) == Some(name)
            || g.str_attr(t, mf::NamedElement_entityName)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
    }

    /// The processing resource of `t`: a specification, or a container's (unique, or by type).
    fn spec(&self, t: &Target) -> R<ObjId> {
        let o = self.find_as(
            &t.element,
            &[mc::ProcessingResourceSpecification, mc::ResourceContainer],
        )?;
        if self.is(o, mc::ProcessingResourceSpecification) {
            if let Some(ty) = &t.resource_type {
                let rt = self.g().get_ref(
                    o,
                    mf::ProcessingResourceSpecification_activeResourceType_ActiveResourceSpecification,
                );
                if !self.type_is(rt, ty) {
                    return err(format!("{} is not of type {ty}", t.element));
                }
            }
            return Ok(o);
        }
        self.expect(
            o,
            mc::ResourceContainer,
            "processing resource or resource container",
        )?;
        let specs = self.g().get_refs(
            o,
            mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
        );
        let chosen: Vec<ObjId> = specs
            .iter()
            .copied()
            .filter(|&p| match &t.resource_type {
                None => true,
                Some(ty) => self.type_is(
                    self.g().get_ref(
                        p,
                        mf::ProcessingResourceSpecification_activeResourceType_ActiveResourceSpecification,
                    ),
                    ty,
                ),
            })
            .collect();
        match chosen.as_slice() {
            [p] => Ok(*p),
            [] => err(format!(
                "{} has no processing resource{}",
                t.element,
                t.resource_type
                    .as_ref()
                    .map(|x| format!(" of type {x}"))
                    .unwrap_or_default()
            )),
            _ => err(format!(
                "{} has {} processing resources: name the type (`{}:CPU`)",
                t.element,
                chosen.len(),
                t.element
            )),
        }
    }

    /// The communication specification of a linking resource (or the specification itself).
    fn link_spec(&self, r: &str) -> R<ObjId> {
        let o = self.find_as(
            r,
            &[
                mc::CommunicationLinkResourceSpecification,
                mc::LinkingResource,
            ],
        )?;
        if self.is(o, mc::CommunicationLinkResourceSpecification) {
            return Ok(o);
        }
        self.expect(o, mc::LinkingResource, "linking resource")?;
        self.g()
            .get_ref(
                o,
                mf::LinkingResource_communicationLinkResourceSpecifications_LinkingResource,
            )
            .ok_or_else(|| EditError(format!("{r} has no communication specification")))
    }

    /// The workload of a usage scenario (or the workload itself), of class `c`.
    fn workload(&self, r: &str, c: ClassId, what: &str) -> R<ObjId> {
        let mut o = self.find_as(r, &[mc::UsageScenario, c])?;
        if self.is(o, mc::UsageScenario) {
            o = self
                .g()
                .get_ref(o, mf::UsageScenario_workload_UsageScenario)
                .ok_or_else(|| EditError(format!("{r} has no workload")))?;
        }
        self.expect(o, c, what)?;
        Ok(o)
    }

    fn op(&mut self, op: &EditOp) -> R<()> {
        match op {
            EditOp::Options(_) => Ok(()),
            EditOp::CloneMonitors {
                template,
                container,
            } => {
                let t = self.find_as(template, &[mc::ResourceContainer])?;
                self.expect(t, mc::ResourceContainer, "resource container")?;
                let c = self.find_as(container, &[mc::ResourceContainer])?;
                self.expect(c, mc::ResourceContainer, "resource container")?;
                let prefix = format!("{}_mon", self.g().id(c).unwrap_or("container"));
                let n = self.e.clone_monitors(t, c, &prefix)?;
                if n.is_empty() {
                    return err(format!("{template} has no monitors to clone"));
                }
                Ok(())
            }
            EditOp::AddDemand { target, spec } => {
                let a = self.find_as(&target.element, &[mc::InternalAction])?;
                self.expect(a, mc::InternalAction, "internal action")?;
                let Some(ty) = &target.resource_type else {
                    return err("add-demand: name the resource type (`ACTION:CPU`)");
                };
                let Some(&(t, ..)) = self
                    .types
                    .iter()
                    .find(|x| &*x.1 == ty.as_str() || x.2.eq_ignore_ascii_case(ty))
                else {
                    return err(format!("no resource type {ty}"));
                };
                let st = self.e.store();
                let d = st.create(mc::ParametricResourceDemand)?;
                st.set(
                    d,
                    mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
                    Datum::Obj(t),
                )?;
                st.add(
                    a,
                    mf::AbstractInternalControlFlowAction_resourceDemand_Action,
                    Datum::Obj(d),
                    None,
                )?;
                self.e.set_spec(
                    d,
                    mf::ParametricResourceDemand_specification_ParametericResourceDemand,
                    spec,
                )
            }
            EditOp::Rate { target, spec } => {
                let p = self.spec(target)?;
                self.e.set_rate(p, spec)
            }
            EditOp::Replicas { target, n } => {
                let p = self.spec(target)?;
                self.e.store().set(
                    p,
                    mf::ProcessingResourceSpecification_numberOfReplicas,
                    Datum::Int(*n),
                )?;
                Ok(())
            }
            EditOp::Latency { link, spec } => {
                let s = self.link_spec(link)?;
                self.e.set_spec(
                    s,
                    mf::CommunicationLinkResourceSpecification_latency_CommunicationLinkResourceSpecification,
                    spec,
                )
            }
            EditOp::Throughput { link, spec } => {
                let s = self.link_spec(link)?;
                self.e.set_spec(
                    s,
                    mf::CommunicationLinkResourceSpecification_throughput_CommunicationLinkResourceSpecification,
                    spec,
                )
            }
            EditOp::Demand { target, spec } => {
                let a = self.find_as(
                    &target.element,
                    &[mc::InternalAction, mc::ParametricResourceDemand],
                )?;
                let d = if self.is(a, mc::ParametricResourceDemand) {
                    a
                } else {
                    let demands = self.g().get_refs(
                        a,
                        mf::AbstractInternalControlFlowAction_resourceDemand_Action,
                    );
                    let chosen: Vec<ObjId> = demands
                        .iter()
                        .copied()
                        .filter(|&d| match &target.resource_type {
                            None => true,
                            Some(ty) => self.type_is(
                                self.g().get_ref(
                                    d,
                                    mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
                                ),
                                ty,
                            ),
                        })
                        .collect();
                    match chosen.as_slice() {
                        [d] => *d,
                        [] => {
                            return err(format!(
                                "{} has no resource demand{}",
                                target.element,
                                target
                                    .resource_type
                                    .as_ref()
                                    .map(|x| format!(" of type {x}"))
                                    .unwrap_or_default()
                            ));
                        }
                        _ => {
                            return err(format!(
                                "{} has {} resource demands: name the type (`{}:CPU`)",
                                target.element,
                                chosen.len(),
                                target.element
                            ));
                        }
                    }
                };
                self.e.set_spec(
                    d,
                    mf::ParametricResourceDemand_specification_ParametericResourceDemand,
                    spec,
                )
            }
            EditOp::Probability { transition, p } => {
                let t = self.find_as(
                    transition,
                    &[mc::ProbabilisticBranchTransition, mc::BranchTransition],
                )?;
                let f = if self.is(t, mc::ProbabilisticBranchTransition) {
                    mf::ProbabilisticBranchTransition_branchProbability
                } else if self.is(t, mc::BranchTransition) {
                    mf::BranchTransition_branchProbability
                } else {
                    return err(format!(
                        "{} is a {}, not a branch transition",
                        self.g().describe(t),
                        self.g()[t].class.name()
                    ));
                };
                self.e.set_double(t, f, *p)
            }
            EditOp::ThinkTime { workload, spec } => {
                let w = self.workload(workload, mc::ClosedWorkload, "closed workload")?;
                self.e
                    .set_spec(w, mf::ClosedWorkload_thinkTime_ClosedWorkload, spec)
            }
            EditOp::InterArrival { workload, spec } => {
                let w = self.workload(workload, mc::OpenWorkload, "open workload")?;
                self.e
                    .set_spec(w, mf::OpenWorkload_interArrivalTime_OpenWorkload, spec)
            }
            EditOp::ArrivalRate { workload, rate } => {
                let w = self.workload(workload, mc::OpenWorkload, "open workload")?;
                let mut s = String::from("Exp(");
                simoxide_sim::javafmt::push_double(&mut s, *rate);
                s.push(')');
                self.e
                    .set_spec(w, mf::OpenWorkload_interArrivalTime_OpenWorkload, &s)
            }
            EditOp::Population { workload, n } => {
                let w = self.workload(workload, mc::ClosedWorkload, "closed workload")?;
                self.e
                    .store()
                    .set(w, mf::ClosedWorkload_population, Datum::Int(*n))?;
                Ok(())
            }
            EditOp::LoopCount { action, spec } => {
                let a = self.find_as(action, &[mc::LoopAction, mc::Loop])?;
                let f = if self.is(a, mc::LoopAction) {
                    mf::LoopAction_iterationCount_LoopAction
                } else if self.is(a, mc::Loop) {
                    mf::Loop_loopIteration_Loop
                } else {
                    return err(format!(
                        "{} is a {}, not a loop",
                        self.g().describe(a),
                        self.g()[a].class.name()
                    ));
                };
                self.e.set_spec(a, f, spec)
            }
            EditOp::Set {
                target,
                feature,
                value,
            } => {
                let o = self.find(target)?;
                let class = self.g()[o].class;
                let f = class.feature(feature).ok_or_else(|| {
                    EditError(format!("a {} has no feature `{feature}`", class.name()))
                })?;
                self.set(o, f, value)
            }
            EditOp::AddContainer { template, id, name } => {
                let t = self.find_as(template, &[mc::ResourceContainer])?;
                self.expect(t, mc::ResourceContainer, "resource container")?;
                if self.e.find(id).is_some() {
                    return err(format!("an element with id {id} exists already"));
                }
                let c = self
                    .e
                    .clone_container(t, id, name.as_deref().unwrap_or(id))?;
                for link in self.e.links_of(t) {
                    self.e.connect(link, c)?;
                }
                Ok(())
            }
            EditOp::Connect { link, container } => {
                let l = self.find_as(link, &[mc::LinkingResource])?;
                self.expect(l, mc::LinkingResource, "linking resource")?;
                let c = self.find_as(container, &[mc::ResourceContainer])?;
                self.expect(c, mc::ResourceContainer, "resource container")?;
                self.e.connect(l, c)
            }
            EditOp::AddAssembly { template, id, name } => {
                let t = self.find_as(template, &[mc::AssemblyContext])?;
                self.expect(t, mc::AssemblyContext, "assembly context")?;
                if self.e.find(id).is_some() {
                    return err(format!("an element with id {id} exists already"));
                }
                self.e
                    .clone_assembly(t, id, name.as_deref().unwrap_or(id))
                    .map(|_| ())
            }
            EditOp::Allocate {
                assembly,
                container,
                id,
            } => {
                let a = self.find_as(assembly, &[mc::AssemblyContext])?;
                self.expect(a, mc::AssemblyContext, "assembly context")?;
                let c = self.find_as(container, &[mc::ResourceContainer])?;
                self.expect(c, mc::ResourceContainer, "resource container")?;
                if self.e.find(id).is_some() {
                    return err(format!("an element with id {id} exists already"));
                }
                self.e.allocate(a, c, id).map(|_| ())
            }
            EditOp::RemoveContainer { container } => {
                let c = self.find_as(container, &[mc::ResourceContainer])?;
                self.expect(c, mc::ResourceContainer, "resource container")?;
                self.e.remove_from_environment(c)
            }
            EditOp::Delete { target } => {
                let o = self.find(target)?;
                self.e.delete(o);
                Ok(())
            }
        }
    }

    /// `set`: an attribute, a random variable's specification, or a reference.
    fn set(&mut self, o: ObjId, f: FeatureId, value: &str) -> R<()> {
        let d = f.def();
        if d.many {
            return err(format!("`{}` is many-valued", f.name()));
        }
        if let Some(kind) = f.data_kind() {
            let v = simoxide_model::raw::parse_value(kind, value)
                .ok_or_else(|| EditError(format!("`{value}` is not a valid {}", f.name())))?;
            let datum = match (kind, &v) {
                (DataKind::Str, Value::Str(s)) => Datum::Str(s),
                (DataKind::Int, Value::Int(i)) => Datum::Int(
                    i32::try_from(*i).map_err(|_| EditError(format!("{i}: out of range")))?,
                ),
                (DataKind::Long, Value::Int(i)) => Datum::Long(*i),
                (DataKind::Double, Value::Double(x)) => Datum::Double(*x),
                (DataKind::Bool, Value::Bool(b)) => Datum::Bool(*b),
                _ => {
                    return err(format!(
                        "`set` does not support the type of `{}` (use a QVTo transformation)",
                        f.name()
                    ));
                }
            };
            self.e.store().set(o, f, datum)?;
            return Ok(());
        }
        let target = f.target();
        if f.is_containment() && target.is_some_and(|t| mc::PCMRandomVariable.is_a(t)) {
            return self.e.set_spec(o, f, value);
        }
        if f.is_containment() {
            return err(format!(
                "`{}` contains elements: use a QVTo transformation",
                f.name()
            ));
        }
        let v = self.find(value.trim())?;
        self.e.set_ref(o, f, v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_format() {
        let f = ChangeFile::parse(
            "# a comment\n\
             rate Server1:CPU 2000 * 2\n\
             demand \"My Action\" Exp(0.5)\n\
             probability _t1 0.25\n\
             arrival-rate _s 4\n\
             add-container Server1 _s3 \"Server 3\"\n\
             allocate _ac _s3 _alc\n\
             set _x entityName New Name\n",
        )
        .unwrap();
        assert_eq!(f.ops.len(), 7);
        assert_eq!(
            f.ops[0],
            (
                2,
                EditOp::Rate {
                    target: Target {
                        element: "Server1".into(),
                        resource_type: Some("CPU".into())
                    },
                    spec: "2000 * 2".into()
                }
            )
        );
        assert_eq!(
            f.ops[1].1,
            EditOp::Demand {
                target: Target {
                    element: "My Action".into(),
                    resource_type: None
                },
                spec: "Exp(0.5)".into()
            }
        );
        assert_eq!(
            f.ops[4].1,
            EditOp::AddContainer {
                template: "Server1".into(),
                id: "_s3".into(),
                name: Some("Server 3".into())
            }
        );
        assert_eq!(
            f.ops[6].1,
            EditOp::Set {
                target: "_x".into(),
                feature: "entityName".into(),
                value: "New Name".into()
            }
        );
        for (bad, line) in [
            ("rate Server1", 1),
            ("\nfrobnicate x", 2),
            ("probability _t abc", 1),
            ("population _w 1.5", 1),
            ("allocate _a _b", 1),
            ("connect a b c", 1),
            ("rate \"x 1", 1),
            ("arrival-rate _s 0", 1),
        ] {
            match ChangeFile::parse(bad) {
                Err(ChangeFileError::Line(l, _)) => assert_eq!(l, line, "{bad}"),
                o => panic!("{bad}: {o:?}"),
            }
        }
    }
}
