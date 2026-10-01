//! Canonical, line-oriented JSON dump of a loaded object graph. The Java oracle
//! (`reference/oracles/pcm-model`) writes the same format from real EMF; equality of the two dumps
//! is the proof that the loader reads models exactly like EMF.
//!
//! Format: one line per resource header and per object, resources sorted by name, objects in EMF
//! tree order:
//! `{"resource":"name","roots":N}`
//! `{"path":"//@a.0","id":"_x","type":"pkg:Class","attrs":{..},"refs":{..}}`
//! - `attrs`: every persisted attribute except the ID, with its effective value (defaults
//!   included); doubles as Java `Double.toString` strings, enums as literal names;
//! - `refs`: every set, persisted, non-containment reference; targets as `resource#fragment`
//!   (fragment = ID or EMF path), unresolved proxies as `?file#fragment`.

use crate::meta::{DataKind, FeatureId};
use crate::raw::{Graph, ObjId, ResId, Value};
use std::fmt::Write;

/// Java `Double.toString` (JDK 19+, shortest repr).
pub fn java_double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if d == 0.0 {
        return if d.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let e = format!("{:e}", d.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if d < 0.0 { "-" } else { "" };
    let a = d.abs();
    if (1e-3..1e7).contains(&a) {
        // plain notation
        let point = exp + 1; // digits before the decimal point
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp)
    }
}

pub(crate) fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn value(out: &mut String, f: FeatureId, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Str(s) | Value::Other(s) => json_str(out, s),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Double(d) => json_str(out, &java_double_to_string(*d)),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Enum(e) => match f.data_kind() {
            Some(DataKind::Enum(en)) => json_str(out, en.def().literals[*e as usize].name),
            _ => json_str(out, &e.to_string()),
        },
    }
}

/// Reference target as it appears in the canonical dump.
pub fn target(g: &Graph, t: ObjId) -> String {
    if let Some(p) = &g[t].proxy {
        let (key, frag) = p.split_once('#').unwrap_or((p, ""));
        let base = key.rsplit('/').next().unwrap_or(key);
        return format!("?{base}#{frag}");
    }
    g.describe_ref(t)
}

/// Attributes whose getter the generated implementation overrides with a computed value (the
/// stored value is never what EMF reports), excluded from the comparison.
pub fn computed_attribute(f: FeatureId) -> bool {
    f.def().owner == crate::meta::class::MeasuringPoint
        && matches!(
            f.name(),
            "stringRepresentation" | "resourceURIRepresentation"
        )
}

/// One object's canonical line (without newline).
pub fn object_line(g: &Graph, o: ObjId) -> String {
    let obj = &g[o];
    let class = obj.class;
    let mut s = String::with_capacity(256);
    s.push_str("{\"path\":");
    json_str(&mut s, &g.path_fragment(o));
    s.push_str(",\"id\":");
    match g.id(o) {
        Some(id) => json_str(&mut s, id),
        None if g.has_generated_id(o) => s.push_str("\"<generated>\""),
        None => s.push_str("null"),
    }
    s.push_str(",\"type\":");
    json_str(&mut s, &class.qualified_name());
    s.push_str(",\"attrs\":{");
    let idattr = class.def().id_attribute;
    let mut first = true;
    for f in class.features() {
        if !f.is_attribute() || !f.is_persistent() || Some(f) == idattr || computed_attribute(f) {
            continue;
        }
        if !first {
            s.push(',');
        }
        first = false;
        json_str(&mut s, f.name());
        s.push(':');
        if f.def().many {
            s.push('[');
            for (i, v) in g.attrs(o, f).iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                value(&mut s, f, v);
            }
            s.push(']');
        } else {
            value(&mut s, f, &g.attr(o, f));
        }
    }
    s.push_str("},\"refs\":{");
    let mut first = true;
    for f in class.features() {
        if f.is_attribute() || !f.is_cross_reference() {
            continue;
        }
        let ts = g.get_refs(o, f);
        if ts.is_empty() {
            continue;
        }
        if !first {
            s.push(',');
        }
        first = false;
        json_str(&mut s, f.name());
        s.push(':');
        if f.def().many {
            s.push('[');
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                json_str(&mut s, &target(g, *t));
            }
            s.push(']');
        } else {
            json_str(&mut s, &target(g, ts[0]));
        }
    }
    s.push_str("}}");
    s
}

/// Per resource: its name, or `name~k` for the k-th (k >= 1) further resource with the same name
/// in dump order (the same file loaded under two URIs).
pub fn resource_labels(g: &Graph) -> Vec<String> {
    let mut rs: Vec<usize> = (0..g.resources.len())
        .filter(|r| !g.resources[*r].roots.is_empty())
        .collect();
    rs.sort_by(|a, b| g.resources[*a].name.cmp(&g.resources[*b].name));
    let mut labels: Vec<String> = g.resources.iter().map(|r| r.name.clone()).collect();
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for r in rs {
        let n = seen.entry(&g.resources[r].name).or_insert(0);
        if *n > 0 {
            labels[r] = format!("{}~{}", g.resources[r].name, n);
        }
        *n += 1;
    }
    labels
}

/// Canonical dump of all (non-empty) resources. Bundled `pathmap:` resources are only included
/// with `include_bundled` (the oracle dumps them once, in its `_bundled` entry).
pub fn dump(g: &Graph, include_bundled: bool) -> String {
    let mut rs: Vec<ResId> = (0..g.resources.len() as u32)
        .map(ResId)
        .filter(|r| {
            let res = &g.resources[r.0 as usize];
            !res.roots.is_empty() && (include_bundled || !res.name.starts_with("pathmap:"))
        })
        .collect();
    rs.sort_by(|a, b| {
        g.resources[a.0 as usize]
            .name
            .cmp(&g.resources[b.0 as usize].name)
    });
    let mut out = String::new();
    for r in rs {
        let res = &g.resources[r.0 as usize];
        out.push_str("{\"resource\":");
        json_str(&mut out, &res.name);
        let _ = writeln!(out, ",\"roots\":{}}}", res.roots.len());
        for o in g.all_contents(r) {
            out.push_str(&object_line(g, o));
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// typed-model dump

use crate::model::*;

enum J<'a> {
    S(Option<&'a str>),
    I(i64),
    D(f64),
    B(bool),
    E(&'static str),
}

enum R {
    One(Option<ObjId>),
    Many(Vec<ObjId>),
}

struct W<'a> {
    m: &'a Model,
    labels: Vec<String>,
    out: String,
}

type Attrs<'a> = Vec<(&'static str, J<'a>)>;
type Refs = Vec<(&'static str, R)>;
type Contains = Vec<(&'static str, Vec<ObjId>)>;

fn opt<T: Copy>(x: Option<T>, f: impl Fn(T) -> ObjId) -> R {
    R::One(x.map(f))
}

fn many<T: Copy>(xs: &[T], f: impl Fn(T) -> ObjId) -> R {
    R::Many(xs.iter().map(|x| f(*x)).collect())
}

impl W<'_> {
    fn g(&self) -> &Graph {
        &self.m.graph
    }

    #[allow(clippy::too_many_arguments)]
    fn line_at(
        &mut self,
        res: &str,
        path: &str,
        ty: &str,
        id: &str,
        attrs: Attrs,
        refs: Refs,
        contains: Contains,
    ) {
        let mut s = String::from("{\"res\":");
        json_str(&mut s, res);
        s.push_str(",\"path\":");
        json_str(&mut s, path);
        s.push_str(",\"type\":");
        json_str(&mut s, ty);
        s.push_str(",\"id\":");
        json_str(&mut s, id);
        s.push_str(",\"attrs\":{");
        for (i, (k, v)) in attrs.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            json_str(&mut s, k);
            s.push(':');
            match v {
                J::S(None) => s.push_str("null"),
                J::S(Some(x)) => json_str(&mut s, x),
                J::I(i) => {
                    let _ = write!(s, "{i}");
                }
                J::D(d) => json_str(&mut s, &java_double_to_string(*d)),
                J::B(b) => s.push_str(if *b { "true" } else { "false" }),
                J::E(e) => json_str(&mut s, e),
            }
        }
        s.push_str("},\"refs\":{");
        for (i, (k, v)) in refs.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            json_str(&mut s, k);
            s.push(':');
            match v {
                R::One(None) => s.push_str("null"),
                R::One(Some(t)) => json_str(&mut s, &target(self.g(), *t)),
                R::Many(ts) => {
                    s.push('[');
                    for (j, t) in ts.iter().enumerate() {
                        if j > 0 {
                            s.push(',');
                        }
                        json_str(&mut s, &target(self.g(), *t));
                    }
                    s.push(']');
                }
            }
        }
        s.push_str("},\"contains\":{");
        for (i, (k, v)) in contains.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            json_str(&mut s, k);
            s.push_str(":[");
            for (j, c) in v.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                json_str(&mut s, &self.g().path_fragment(*c));
            }
            s.push(']');
        }
        s.push_str("}}\n");
        self.out.push_str(&s);
    }

    fn line(&mut self, o: ObjId, ty: &str, id: &str, attrs: Attrs, refs: Refs, contains: Contains) {
        let g = &self.m.graph;
        let res = self.labels[g[o].resource.0 as usize].clone();
        let path = g.path_fragment(o);
        self.line_at(&res, &path, ty, id, attrs, refs, contains);
    }

    fn rv(&mut self, r: &RandomVar) {
        if let Some(o) = r.obj {
            self.line(
                o,
                "core:PCMRandomVariable",
                "",
                vec![("specification", J::S(Some(&r.spec)))],
                vec![],
                vec![],
            );
        }
    }

    fn rvs(r: &RandomVar) -> Vec<ObjId> {
        r.obj.into_iter().collect()
    }

    fn usages(&mut self, us: &[VariableUsage]) {
        for u in us {
            let vcs: Vec<ObjId> = u.characterisations.iter().map(|c| c.obj).collect();
            self.line(
                u.obj,
                "parameter:VariableUsage",
                "",
                vec![],
                vec![],
                vec![("variableCharacterisation_VariableUsage", vcs)],
            );
            for c in &u.characterisations {
                self.line(
                    c.obj,
                    "parameter:VariableCharacterisation",
                    "",
                    vec![("type", J::E(c.kind.name()))],
                    vec![],
                    vec![("specification_VariableCharacterisation", Self::rvs(&c.spec))],
                );
                self.rv(&c.spec);
            }
            // the reference chain, with paths computed from the names only
            let g = &self.m.graph;
            let res = self.labels[g[u.obj].resource.0 as usize].clone();
            let mut path = format!("{}/@namedReference__VariableUsage", g.path_fragment(u.obj));
            for (i, n) in u.reference.iter().enumerate() {
                let last = i + 1 == u.reference.len();
                let ty = if last {
                    "stoex:VariableReference"
                } else {
                    "stoex:NamespaceReference"
                };
                self.line_at(
                    &res,
                    &path,
                    ty,
                    "",
                    vec![("referenceName", J::S(Some(n)))],
                    vec![],
                    vec![],
                );
                path.push_str("/@innerReference_NamespaceReference");
            }
        }
    }

    fn uobjs(us: &[VariableUsage]) -> Vec<ObjId> {
        us.iter().map(|u| u.obj).collect()
    }

    fn run(&mut self) {
        let m = self.m;
        let comp = |x: ComponentId| m.components[x].obj;
        let iface = |x: InterfaceId| m.interfaces[x].obj;
        let sig = |x: SignatureId| m.signatures[x].obj;
        let par = |x: ParameterId| m.parameters[x].obj;
        let dt = |x: DataTypeId| m.data_types[x].obj;
        let role = |x: RoleId| m.roles[x].obj;
        let pres = |x: PassiveResourceId| m.passive_resources[x].obj;
        let seff = |x: SeffId| m.seffs[x].obj;
        let beh = |x: BehaviourId| m.behaviours[x].obj;
        let act = |x: ActionId| m.actions[x].obj;
        let ac = |x: AssemblyContextId| m.assembly_contexts[x].obj;
        let con = |x: ConnectorId| m.connectors[x].obj;
        let sys = |x: SystemId| m.systems[x].obj;
        let env = |x: ResourceEnvironmentId| m.resource_environments[x].obj;
        let cont = |x: ContainerId| m.containers[x].obj;
        let proc = |x: ProcessingResourceId| m.processing_resources[x].obj;
        let link = |x: LinkingResourceId| m.linking_resources[x].obj;
        let rtype = |x: ResourceTypeId| m.resource_types[x].obj;
        let pol = |x: SchedulingPolicyId| m.scheduling_policies[x].obj;
        let riface = |x: ResourceInterfaceId| m.resource_interfaces[x].obj;
        let rsig = |x: ResourceSignatureId| m.resource_signatures[x].obj;
        let alc = |x: AllocationContextId| m.allocation_contexts[x].obj;
        let scen = |x: UsageScenarioId| m.usage_scenarios[x].obj;
        let sb = |x: ScenarioBehaviourId| m.scenario_behaviours[x].obj;
        let ua = |x: UserActionId| m.user_actions[x].obj;
        let mon = |x: MonitorId| m.monitors[x].obj;
        let mp = |x: MeasuringPointId| m.measuring_points[x].obj;
        let met = |x: MetricId| m.metrics[x].obj;

        for r in m.repositories.iter() {
            let c: Vec<ObjId> = r.components.iter().map(|x| comp(*x)).collect();
            let i: Vec<ObjId> = r.interfaces.iter().map(|x| iface(*x)).collect();
            let d: Vec<ObjId> = r.data_types.iter().map(|x| dt(*x)).collect();
            self.line(
                r.obj,
                "repository:Repository",
                &r.id,
                vec![
                    ("entityName", J::S(Some(&r.name))),
                    ("repositoryDescription", J::S(r.description.as_deref())),
                ],
                vec![],
                vec![
                    ("components__Repository", c),
                    ("interfaces__Repository", i),
                    ("dataTypes__Repository", d),
                ],
            );
        }
        for c in m.components.iter() {
            let ty = self.g()[c.obj].class.qualified_name();
            let mut attrs: Attrs = vec![("entityName", J::S(Some(&c.name)))];
            if matches!(
                c.kind,
                ComponentKind::Basic { .. } | ComponentKind::Composite { .. }
            ) {
                attrs.push((
                    "componentType",
                    J::E(match c.component_type {
                        ComponentType::Business => "BUSINESS_COMPONENT",
                        ComponentType::Infrastructure => "INFRASTRUCTURE_COMPONENT",
                    }),
                ));
            }
            let mut contains: Contains = vec![
                (
                    "providedRoles_InterfaceProvidingEntity",
                    c.provided_roles.iter().map(|x| role(*x)).collect(),
                ),
                (
                    "requiredRoles_InterfaceRequiringEntity",
                    c.required_roles.iter().map(|x| role(*x)).collect(),
                ),
                (
                    "resourceRequiredRoles__ResourceInterfaceRequiringEntity",
                    c.resource_required_roles.iter().map(|x| role(*x)).collect(),
                ),
            ];
            match &c.kind {
                ComponentKind::Basic {
                    seffs,
                    passive_resources,
                } => {
                    contains.push((
                        "serviceEffectSpecifications__BasicComponent",
                        seffs.iter().map(|x| seff(*x)).collect(),
                    ));
                    contains.push((
                        "passiveResource_BasicComponent",
                        passive_resources.iter().map(|x| pres(*x)).collect(),
                    ));
                }
                ComponentKind::Composite { structure } | ComponentKind::SubSystem { structure } => {
                    let s = &m.structures[*structure];
                    contains.push((
                        "assemblyContexts__ComposedStructure",
                        s.assembly_contexts.iter().map(|x| ac(*x)).collect(),
                    ));
                    contains.push((
                        "connectors__ComposedStructure",
                        s.connectors.iter().map(|x| con(*x)).collect(),
                    ));
                }
                _ => {}
            }
            if !matches!(
                c.kind,
                ComponentKind::CompleteType
                    | ComponentKind::ProvidesType
                    | ComponentKind::SubSystem { .. }
            ) {
                contains.push((
                    "componentParameterUsage_ImplementationComponentType",
                    Self::uobjs(&c.parameter_usages),
                ));
            }
            self.line(c.obj, &ty, &c.id, attrs, vec![], contains);
            self.usages(&c.parameter_usages);
        }
        for i in m.interfaces.iter() {
            let ty = self.g()[i.obj].class.qualified_name();
            let feat = match i.kind {
                InterfaceKind::Operation => "signatures__OperationInterface",
                InterfaceKind::Infrastructure => {
                    "infrastructureSignatures__InfrastructureInterface"
                }
                InterfaceKind::EventGroup => "eventTypes__EventGroup",
            };
            self.line(
                i.obj,
                &ty,
                &i.id,
                vec![("entityName", J::S(Some(&i.name)))],
                vec![("parentInterfaces__Interface", many(&i.parents, iface))],
                vec![(feat, i.signatures.iter().map(|x| sig(*x)).collect())],
            );
        }
        for s in m.signatures.iter() {
            let ty = self.g()[s.obj].class.qualified_name();
            let (feat, refs) = match s.kind {
                SignatureKind::Operation => (
                    "parameters__OperationSignature",
                    vec![("returnType__OperationSignature", opt(s.return_type, dt))],
                ),
                SignatureKind::Infrastructure => ("parameters__InfrastructureSignature", vec![]),
                SignatureKind::EventType => ("parameter__EventType", vec![]),
            };
            self.line(
                s.obj,
                &ty,
                &s.id,
                vec![("entityName", J::S(Some(&s.name)))],
                refs,
                vec![(feat, s.parameters.iter().map(|x| par(*x)).collect())],
            );
        }
        for p in m.parameters.iter() {
            let modifier = match p.modifier {
                ParameterModifier::None => "none",
                ParameterModifier::In => "in",
                ParameterModifier::Out => "out",
                ParameterModifier::InOut => "inout",
            };
            self.line(
                p.obj,
                "repository:Parameter",
                "",
                vec![
                    ("parameterName", J::S(Some(&p.name))),
                    ("modifier__Parameter", J::E(modifier)),
                ],
                vec![("dataType__Parameter", opt(p.data_type, dt))],
                vec![],
            );
        }
        for d in m.data_types.iter() {
            let ty = self.g()[d.obj].class.qualified_name();
            match &d.kind {
                DataTypeKind::Primitive(p) => {
                    let n = match p {
                        PrimitiveType::Int => "INT",
                        PrimitiveType::String => "STRING",
                        PrimitiveType::Bool => "BOOL",
                        PrimitiveType::Double => "DOUBLE",
                        PrimitiveType::Char => "CHAR",
                        PrimitiveType::Byte => "BYTE",
                        PrimitiveType::Long => "LONG",
                    };
                    self.line(d.obj, &ty, &d.id, vec![("type", J::E(n))], vec![], vec![]);
                }
                DataTypeKind::Collection { inner } => self.line(
                    d.obj,
                    &ty,
                    &d.id,
                    vec![("entityName", J::S(Some(&d.name)))],
                    vec![("innerType_CollectionDataType", opt(*inner, dt))],
                    vec![],
                ),
                DataTypeKind::Composite { parents, inner } => {
                    self.line(
                        d.obj,
                        &ty,
                        &d.id,
                        vec![("entityName", J::S(Some(&d.name)))],
                        vec![("parentType_CompositeDataType", many(parents, dt))],
                        vec![(
                            "innerDeclaration_CompositeDataType",
                            inner.iter().map(|x| x.obj).collect(),
                        )],
                    );
                    for x in inner {
                        self.line(
                            x.obj,
                            "repository:InnerDeclaration",
                            "",
                            vec![("entityName", J::S(Some(&x.name)))],
                            vec![("datatype_InnerDeclaration", opt(x.data_type, dt))],
                            vec![],
                        );
                    }
                }
            }
        }
        for r in m.roles.iter() {
            let ty = self.g()[r.obj].class.qualified_name();
            let refs = match r.kind {
                RoleKind::OperationProvided => vec![(
                    "providedInterface__OperationProvidedRole",
                    opt(r.interface, iface),
                )],
                RoleKind::OperationRequired => vec![(
                    "requiredInterface__OperationRequiredRole",
                    opt(r.interface, iface),
                )],
                RoleKind::InfrastructureProvided => {
                    vec![(
                        "providedInterface__InfrastructureProvidedRole",
                        opt(r.interface, iface),
                    )]
                }
                RoleKind::InfrastructureRequired => {
                    vec![(
                        "requiredInterface__InfrastructureRequiredRole",
                        opt(r.interface, iface),
                    )]
                }
                RoleKind::SinkProvided => vec![("eventGroup__SinkRole", opt(r.interface, iface))],
                RoleKind::SourceRequired => {
                    vec![("eventGroup__SourceRole", opt(r.interface, iface))]
                }
                RoleKind::ResourceProvided => {
                    vec![(
                        "providedResourceInterface__ResourceProvidedRole",
                        opt(r.resource_interface, riface),
                    )]
                }
                RoleKind::ResourceRequired => {
                    vec![(
                        "requiredResourceInterface__ResourceRequiredRole",
                        opt(r.resource_interface, riface),
                    )]
                }
            };
            self.line(
                r.obj,
                &ty,
                &r.id,
                vec![("entityName", J::S(Some(&r.name)))],
                refs,
                vec![],
            );
        }
        for p in m.passive_resources.iter() {
            self.line(
                p.obj,
                "repository:PassiveResource",
                &p.id,
                vec![("entityName", J::S(Some(&p.name)))],
                vec![],
                vec![("capacity_PassiveResource", Self::rvs(&p.capacity))],
            );
            self.rv(&p.capacity);
        }
        for s in m.seffs.iter() {
            let b = &m.behaviours[s.behaviour];
            self.line(
                s.obj,
                "seff:ResourceDemandingSEFF",
                &s.id,
                vec![("seffTypeID", J::S(Some(&s.seff_type_id)))],
                vec![("describedService__SEFF", opt(s.signature, sig))],
                vec![
                    ("steps_Behaviour", b.steps.iter().map(|x| act(*x)).collect()),
                    (
                        "resourceDemandingInternalBehaviours",
                        s.internal_behaviours.iter().map(|x| beh(*x)).collect(),
                    ),
                ],
            );
        }
        for b in m.behaviours.iter() {
            if matches!(b.owner, BehaviourOwner::Seff(_)) {
                continue;
            }
            let ty = self.g()[b.obj].class.qualified_name();
            self.line(
                b.obj,
                &ty,
                &b.id,
                vec![],
                vec![],
                vec![("steps_Behaviour", b.steps.iter().map(|x| act(*x)).collect())],
            );
        }
        for a in m.actions.iter() {
            let ty = self.g()[a.obj].class.qualified_name();
            let mut attrs: Attrs = vec![("entityName", J::S(Some(&a.name)))];
            let mut refs: Refs = vec![
                ("predecessor_AbstractAction", opt(a.predecessor, act)),
                ("successor_AbstractAction", opt(a.successor, act)),
            ];
            let mut contains: Contains = Vec::new();
            match &a.kind {
                ActionKind::ExternalCall {
                    signature,
                    role: r,
                    inputs,
                    returns,
                    retry_count,
                } => {
                    attrs.push(("retryCount", J::I(*retry_count)));
                    refs.push(("calledService_ExternalService", opt(*signature, sig)));
                    refs.push(("role_ExternalService", opt(*r, role)));
                    contains.push(("inputVariableUsages__CallAction", Self::uobjs(inputs)));
                    contains.push((
                        "returnVariableUsage__CallReturnAction",
                        Self::uobjs(returns),
                    ));
                    self.usages(inputs);
                    self.usages(returns);
                }
                ActionKind::Branch { transitions } => {
                    contains.push((
                        "branches_Branch",
                        transitions.iter().map(|t| t.obj).collect(),
                    ));
                    for t in transitions {
                        let tty = self.g()[t.obj].class.qualified_name();
                        let (tattrs, tcont): (Attrs, Contains) = match &t.condition {
                            BranchCondition::Probability(p) => (
                                vec![
                                    ("entityName", J::S(Some(&t.name))),
                                    ("branchProbability", J::D(*p)),
                                ],
                                vec![],
                            ),
                            BranchCondition::Guard(g) => (
                                vec![("entityName", J::S(Some(&t.name)))],
                                vec![("branchCondition_GuardedBranchTransition", Self::rvs(g))],
                            ),
                        };
                        let mut tcont = tcont;
                        tcont.push((
                            "branchBehaviour_BranchTransition",
                            t.behaviour.map(beh).into_iter().collect(),
                        ));
                        self.line(t.obj, &tty, &t.id, tattrs, vec![], tcont);
                        if let BranchCondition::Guard(g) = &t.condition {
                            self.rv(g);
                        }
                    }
                }
                ActionKind::Loop { iterations, body } => {
                    contains.push(("iterationCount_LoopAction", Self::rvs(iterations)));
                    contains.push(("bodyBehaviour_Loop", body.map(beh).into_iter().collect()));
                    self.rv(iterations);
                }
                ActionKind::CollectionIterator { parameter, body } => {
                    refs.push(("parameter_CollectionIteratorAction", opt(*parameter, par)));
                    contains.push(("bodyBehaviour_Loop", body.map(beh).into_iter().collect()));
                }
                ActionKind::Fork {
                    asynchronous,
                    synchronisation,
                } => {
                    contains.push((
                        "asynchronousForkedBehaviours_ForkAction",
                        asynchronous.iter().map(|x| beh(*x)).collect(),
                    ));
                    contains.push((
                        "synchronisingBehaviours_ForkAction",
                        synchronisation.iter().map(|s| s.obj).collect(),
                    ));
                    if let Some(sp) = synchronisation {
                        self.line(
                            sp.obj,
                            "seff:SynchronisationPoint",
                            m.graph.id(sp.obj).unwrap_or(""),
                            vec![],
                            vec![],
                            vec![
                                (
                                    "synchronousForkedBehaviours_SynchronisationPoint",
                                    sp.synchronous.iter().map(|x| beh(*x)).collect(),
                                ),
                                (
                                    "outputParameterUsage_SynchronisationPoint",
                                    Self::uobjs(&sp.outputs),
                                ),
                            ],
                        );
                        self.usages(&sp.outputs);
                    }
                }
                ActionKind::Acquire {
                    resource,
                    timeout,
                    timeout_value,
                } => {
                    attrs.push(("timeout", J::B(*timeout)));
                    attrs.push(("timeoutValue", J::D(*timeout_value)));
                    refs.push(("passiveresource_AcquireAction", opt(*resource, pres)));
                }
                ActionKind::Release { resource } => {
                    refs.push(("passiveResource_ReleaseAction", opt(*resource, pres)))
                }
                ActionKind::SetVariable { usages } => {
                    contains.push(("localVariableUsages_SetVariableAction", Self::uobjs(usages)));
                    self.usages(usages);
                }
                ActionKind::InternalCall { behaviour, inputs } => {
                    refs.push((
                        "calledResourceDemandingInternalBehaviour",
                        opt(*behaviour, beh),
                    ));
                    contains.push(("inputVariableUsages__CallAction", Self::uobjs(inputs)));
                    self.usages(inputs);
                }
                ActionKind::EmitEvent {
                    event_type,
                    role: r,
                    inputs,
                } => {
                    refs.push(("eventType__EmitEventAction", opt(*event_type, sig)));
                    refs.push(("sourceRole__EmitEventAction", opt(*r, role)));
                    contains.push(("inputVariableUsages__CallAction", Self::uobjs(inputs)));
                    self.usages(inputs);
                }
                ActionKind::Start
                | ActionKind::Stop
                | ActionKind::Internal
                | ActionKind::Unsupported(_) => {}
            }
            if self.g()[a.obj]
                .class
                .is_a(crate::meta::class::AbstractInternalControlFlowAction)
            {
                contains.push((
                    "resourceDemand_Action",
                    a.resource_demands.iter().map(|d| d.obj).collect(),
                ));
                contains.push((
                    "infrastructureCall__Action",
                    a.infrastructure_calls.iter().map(|d| d.obj).collect(),
                ));
                contains.push((
                    "resourceCall__Action",
                    a.resource_calls.iter().map(|d| d.obj).collect(),
                ));
            }
            self.line(a.obj, &ty, &a.id, attrs, refs, contains);
            for d in &a.resource_demands {
                self.line(
                    d.obj,
                    "seff_performance:ParametricResourceDemand",
                    "",
                    vec![],
                    vec![(
                        "requiredResource_ParametricResourceDemand",
                        opt(d.resource_type, rtype),
                    )],
                    vec![(
                        "specification_ParametericResourceDemand",
                        Self::rvs(&d.spec),
                    )],
                );
                self.rv(&d.spec);
            }
            for x in &a.infrastructure_calls {
                self.line(
                    x.obj,
                    "seff_performance:InfrastructureCall",
                    m.graph.id(x.obj).unwrap_or(""),
                    vec![],
                    vec![
                        ("signature__InfrastructureCall", opt(x.signature, sig)),
                        ("requiredRole__InfrastructureCall", opt(x.role, role)),
                    ],
                    vec![
                        (
                            "numberOfCalls__InfrastructureCall",
                            Self::rvs(&x.number_of_calls),
                        ),
                        ("inputVariableUsages__CallAction", Self::uobjs(&x.inputs)),
                    ],
                );
                self.rv(&x.number_of_calls);
                self.usages(&x.inputs);
            }
            for x in &a.resource_calls {
                self.line(
                    x.obj,
                    "seff_performance:ResourceCall",
                    m.graph.id(x.obj).unwrap_or(""),
                    vec![],
                    vec![
                        ("signature__ResourceCall", opt(x.signature, rsig)),
                        ("resourceRequiredRole__ResourceCall", opt(x.role, role)),
                    ],
                    vec![
                        ("numberOfCalls__ResourceCall", Self::rvs(&x.number_of_calls)),
                        ("inputVariableUsages__CallAction", Self::uobjs(&x.inputs)),
                    ],
                );
                self.rv(&x.number_of_calls);
                self.usages(&x.inputs);
            }
        }
        for a in m.assembly_contexts.iter() {
            self.line(
                a.obj,
                "composition:AssemblyContext",
                &a.id,
                vec![("entityName", J::S(Some(&a.name)))],
                vec![(
                    "encapsulatedComponent__AssemblyContext",
                    opt(a.component, comp),
                )],
                vec![(
                    "configParameterUsages__AssemblyContext",
                    Self::uobjs(&a.config_parameters),
                )],
            );
            self.usages(&a.config_parameters);
        }
        for c in m.connectors.iter() {
            let ty = self.g()[c.obj].class.qualified_name();
            let refs: Refs = match &c.kind {
                ConnectorKind::Assembly {
                    requiring,
                    providing,
                    required_role,
                    provided_role,
                } => vec![
                    (
                        "requiringAssemblyContext_AssemblyConnector",
                        opt(*requiring, ac),
                    ),
                    (
                        "providingAssemblyContext_AssemblyConnector",
                        opt(*providing, ac),
                    ),
                    ("requiredRole_AssemblyConnector", opt(*required_role, role)),
                    ("providedRole_AssemblyConnector", opt(*provided_role, role)),
                ],
                ConnectorKind::AssemblyInfrastructure {
                    requiring,
                    providing,
                    required_role,
                    provided_role,
                } => vec![
                    (
                        "requiringAssemblyContext__AssemblyInfrastructureConnector",
                        opt(*requiring, ac),
                    ),
                    (
                        "providingAssemblyContext__AssemblyInfrastructureConnector",
                        opt(*providing, ac),
                    ),
                    (
                        "requiredRole__AssemblyInfrastructureConnector",
                        opt(*required_role, role),
                    ),
                    (
                        "providedRole__AssemblyInfrastructureConnector",
                        opt(*provided_role, role),
                    ),
                ],
                ConnectorKind::ProvidedDelegation {
                    inner_role,
                    outer_role,
                    assembly,
                } => vec![
                    (
                        "innerProvidedRole_ProvidedDelegationConnector",
                        opt(*inner_role, role),
                    ),
                    (
                        "outerProvidedRole_ProvidedDelegationConnector",
                        opt(*outer_role, role),
                    ),
                    (
                        "assemblyContext_ProvidedDelegationConnector",
                        opt(*assembly, ac),
                    ),
                ],
                ConnectorKind::RequiredDelegation {
                    inner_role,
                    outer_role,
                    assembly,
                } => vec![
                    (
                        "innerRequiredRole_RequiredDelegationConnector",
                        opt(*inner_role, role),
                    ),
                    (
                        "outerRequiredRole_RequiredDelegationConnector",
                        opt(*outer_role, role),
                    ),
                    (
                        "assemblyContext_RequiredDelegationConnector",
                        opt(*assembly, ac),
                    ),
                ],
                ConnectorKind::ProvidedInfrastructureDelegation {
                    inner_role,
                    outer_role,
                    assembly,
                } => vec![
                    (
                        "innerProvidedRole__ProvidedInfrastructureDelegationConnector",
                        opt(*inner_role, role),
                    ),
                    (
                        "outerProvidedRole__ProvidedInfrastructureDelegationConnector",
                        opt(*outer_role, role),
                    ),
                    (
                        "assemblyContext__ProvidedInfrastructureDelegationConnector",
                        opt(*assembly, ac),
                    ),
                ],
                ConnectorKind::RequiredInfrastructureDelegation {
                    inner_role,
                    outer_role,
                    assembly,
                } => vec![
                    (
                        "innerRequiredRole__RequiredInfrastructureDelegationConnector",
                        opt(*inner_role, role),
                    ),
                    (
                        "outerRequiredRole__RequiredInfrastructureDelegationConnector",
                        opt(*outer_role, role),
                    ),
                    (
                        "assemblyContext__RequiredInfrastructureDelegationConnector",
                        opt(*assembly, ac),
                    ),
                ],
                ConnectorKind::RequiredResourceDelegation {
                    inner_role,
                    outer_role,
                    assembly,
                } => vec![
                    (
                        "innerRequiredRole__RequiredResourceDelegationConnector",
                        opt(*inner_role, role),
                    ),
                    (
                        "outerRequiredRole__RequiredResourceDelegationConnector",
                        opt(*outer_role, role),
                    ),
                    (
                        "assemblyContext__RequiredResourceDelegationConnector",
                        opt(*assembly, ac),
                    ),
                ],
                ConnectorKind::Unsupported(_) => vec![],
            };
            self.line(
                c.obj,
                &ty,
                &c.id,
                vec![("entityName", J::S(Some(&c.name)))],
                refs,
                vec![],
            );
        }
        for s in m.systems.iter() {
            let st = &m.structures[s.structure];
            self.line(
                s.obj,
                "system:System",
                &s.id,
                vec![("entityName", J::S(Some(&s.name)))],
                vec![],
                vec![
                    (
                        "assemblyContexts__ComposedStructure",
                        st.assembly_contexts.iter().map(|x| ac(*x)).collect(),
                    ),
                    (
                        "connectors__ComposedStructure",
                        st.connectors.iter().map(|x| con(*x)).collect(),
                    ),
                    (
                        "providedRoles_InterfaceProvidingEntity",
                        s.provided_roles.iter().map(|x| role(*x)).collect(),
                    ),
                    (
                        "requiredRoles_InterfaceRequiringEntity",
                        s.required_roles.iter().map(|x| role(*x)).collect(),
                    ),
                    (
                        "resourceRequiredRoles__ResourceInterfaceRequiringEntity",
                        s.resource_required_roles.iter().map(|x| role(*x)).collect(),
                    ),
                ],
            );
        }
        for e in m.resource_environments.iter() {
            self.line(
                e.obj,
                "resourceenvironment:ResourceEnvironment",
                "",
                vec![("entityName", J::S(Some(&e.name)))],
                vec![],
                vec![
                    (
                        "resourceContainer_ResourceEnvironment",
                        e.containers.iter().map(|x| cont(*x)).collect(),
                    ),
                    (
                        "linkingResources__ResourceEnvironment",
                        e.linking_resources.iter().map(|x| link(*x)).collect(),
                    ),
                ],
            );
        }
        for c in m.containers.iter() {
            self.line(
                c.obj,
                "resourceenvironment:ResourceContainer",
                &c.id,
                vec![("entityName", J::S(Some(&c.name)))],
                vec![],
                vec![
                    (
                        "nestedResourceContainers__ResourceContainer",
                        c.nested.iter().map(|x| cont(*x)).collect(),
                    ),
                    (
                        "activeResourceSpecifications_ResourceContainer",
                        c.processing_resources.iter().map(|x| proc(*x)).collect(),
                    ),
                ],
            );
        }
        for p in m.processing_resources.iter() {
            let ty = self.g()[p.obj].class.qualified_name();
            let mut contains: Contains = vec![(
                "processingRate_ProcessingResourceSpecification",
                Self::rvs(&p.processing_rate),
            )];
            if let Some(h) = &p.hdd {
                contains.push(("readProcessingRate", Self::rvs(&h.read)));
                contains.push(("writeProcessingRate", Self::rvs(&h.write)));
            }
            self.line(
                p.obj,
                &ty,
                &p.id,
                vec![
                    ("MTTR", J::D(p.mttr)),
                    ("MTTF", J::D(p.mttf)),
                    ("requiredByContainer", J::B(p.required_by_container)),
                    ("numberOfReplicas", J::I(p.replicas)),
                ],
                vec![
                    ("schedulingPolicy", opt(p.scheduling, pol)),
                    (
                        "activeResourceType_ActiveResourceSpecification",
                        opt(p.resource_type, rtype),
                    ),
                ],
                contains,
            );
            self.rv(&p.processing_rate);
            if let Some(h) = &p.hdd {
                self.rv(&h.read);
                self.rv(&h.write);
            }
        }
        for l in m.linking_resources.iter() {
            let spec = self.g().get_ref(l.obj, crate::meta::feat::LinkingResource_communicationLinkResourceSpecifications_LinkingResource);
            self.line(
                l.obj,
                "resourceenvironment:LinkingResource",
                &l.id,
                vec![("entityName", J::S(Some(&l.name)))],
                vec![(
                    "connectedResourceContainers_LinkingResource",
                    many(&l.connected, cont),
                )],
                vec![(
                    "communicationLinkResourceSpecifications_LinkingResource",
                    spec.into_iter().collect(),
                )],
            );
            if let Some(s) = spec {
                self.line(
                    s,
                    "resourceenvironment:CommunicationLinkResourceSpecification",
                    &l.spec_id,
                    vec![("failureProbability", J::D(l.failure_probability))],
                    vec![(
                        "communicationLinkResourceType_CommunicationLinkResourceSpecification",
                        opt(l.resource_type, rtype),
                    )],
                    vec![
                        (
                            "latency_CommunicationLinkResourceSpecification",
                            Self::rvs(&l.latency),
                        ),
                        (
                            "throughput_CommunicationLinkResourceSpecification",
                            Self::rvs(&l.throughput),
                        ),
                    ],
                );
                self.rv(&l.latency);
                self.rv(&l.throughput);
            }
        }
        for r in m.resource_repositories.iter() {
            self.line(
                r.obj,
                "resourcetype:ResourceRepository",
                "",
                vec![],
                vec![],
                vec![
                    (
                        "availableResourceTypes_ResourceRepository",
                        r.resource_types.iter().map(|x| rtype(*x)).collect(),
                    ),
                    (
                        "schedulingPolicies__ResourceRepository",
                        r.scheduling_policies.iter().map(|x| pol(*x)).collect(),
                    ),
                    (
                        "resourceInterfaces__ResourceRepository",
                        r.interfaces.iter().map(|x| riface(*x)).collect(),
                    ),
                ],
            );
        }
        for t in m.resource_types.iter() {
            let ty = self.g()[t.obj].class.qualified_name();
            self.line(
                t.obj,
                &ty,
                &t.id,
                vec![("entityName", J::S(Some(&t.name)))],
                vec![],
                vec![(
                    "resourceProvidedRoles__ResourceInterfaceProvidingEntity",
                    t.provided_roles.iter().map(|x| role(*x)).collect(),
                )],
            );
        }
        for p in m.scheduling_policies.iter() {
            self.line(
                p.obj,
                "resourcetype:SchedulingPolicy",
                &p.id,
                vec![("entityName", J::S(Some(&p.name)))],
                vec![],
                vec![],
            );
        }
        for i in m.resource_interfaces.iter() {
            self.line(
                i.obj,
                "resourcetype:ResourceInterface",
                &i.id,
                vec![("entityName", J::S(Some(&i.name)))],
                vec![],
                vec![(
                    "resourceSignatures__ResourceInterface",
                    i.signatures.iter().map(|x| rsig(*x)).collect(),
                )],
            );
        }
        for s in m.resource_signatures.iter() {
            self.line(
                s.obj,
                "resourcetype:ResourceSignature",
                &s.id,
                vec![
                    ("entityName", J::S(Some(&s.name))),
                    ("resourceServiceId", J::I(s.service_id)),
                ],
                vec![],
                vec![(
                    "parameter__ResourceSignature",
                    s.parameters.iter().map(|x| par(*x)).collect(),
                )],
            );
        }
        for a in m.allocations.iter() {
            self.line(
                a.obj,
                "allocation:Allocation",
                &a.id,
                vec![("entityName", J::S(Some(&a.name)))],
                vec![
                    ("system_Allocation", opt(a.system, sys)),
                    (
                        "targetResourceEnvironment_Allocation",
                        opt(a.environment, env),
                    ),
                ],
                vec![(
                    "allocationContexts_Allocation",
                    a.contexts.iter().map(|x| alc(*x)).collect(),
                )],
            );
        }
        for a in m.allocation_contexts.iter() {
            self.line(
                a.obj,
                "allocation:AllocationContext",
                &a.id,
                vec![("entityName", J::S(Some(&a.name)))],
                vec![
                    (
                        "resourceContainer_AllocationContext",
                        opt(a.container, cont),
                    ),
                    ("assemblyContext_AllocationContext", opt(a.assembly, ac)),
                ],
                vec![],
            );
        }
        for u in m.usage_models.iter() {
            self.line(
                u.obj,
                "usagemodel:UsageModel",
                "",
                vec![],
                vec![],
                vec![
                    (
                        "usageScenario_UsageModel",
                        u.scenarios.iter().map(|x| scen(*x)).collect(),
                    ),
                    (
                        "userData_UsageModel",
                        u.user_data.iter().map(|x| x.obj).collect(),
                    ),
                ],
            );
            for d in &u.user_data {
                self.line(
                    d.obj,
                    "usagemodel:UserData",
                    "",
                    vec![],
                    vec![("assemblyContext_userData", opt(d.assembly, ac))],
                    vec![("userDataParameterUsages_UserData", Self::uobjs(&d.usages))],
                );
                self.usages(&d.usages);
            }
        }
        for s in m.usage_scenarios.iter() {
            let w = self.g().get_ref(
                s.obj,
                crate::meta::feat::UsageScenario_workload_UsageScenario,
            );
            self.line(
                s.obj,
                "usagemodel:UsageScenario",
                &s.id,
                vec![("entityName", J::S(Some(&s.name)))],
                vec![],
                vec![
                    (
                        "scenarioBehaviour_UsageScenario",
                        s.behaviour.map(sb).into_iter().collect(),
                    ),
                    ("workload_UsageScenario", w.into_iter().collect()),
                ],
            );
            match (&s.workload, w) {
                (
                    Workload::Closed {
                        population,
                        think_time,
                    },
                    Some(w),
                ) => {
                    self.line(
                        w,
                        "usagemodel:ClosedWorkload",
                        "",
                        vec![("population", J::I(*population))],
                        vec![],
                        vec![("thinkTime_ClosedWorkload", Self::rvs(think_time))],
                    );
                    self.rv(think_time);
                }
                (Workload::Open { inter_arrival_time }, Some(w)) => {
                    self.line(
                        w,
                        "usagemodel:OpenWorkload",
                        "",
                        vec![],
                        vec![],
                        vec![(
                            "interArrivalTime_OpenWorkload",
                            Self::rvs(inter_arrival_time),
                        )],
                    );
                    self.rv(inter_arrival_time);
                }
                _ => {}
            }
        }
        for b in m.scenario_behaviours.iter() {
            self.line(
                b.obj,
                "usagemodel:ScenarioBehaviour",
                &b.id,
                vec![("entityName", J::S(Some(&b.name)))],
                vec![],
                vec![(
                    "actions_ScenarioBehaviour",
                    b.actions.iter().map(|x| ua(*x)).collect(),
                )],
            );
        }
        for a in m.user_actions.iter() {
            let ty = self.g()[a.obj].class.qualified_name();
            let mut attrs: Attrs = vec![("entityName", J::S(Some(&a.name)))];
            let mut refs: Refs = vec![
                ("predecessor", opt(a.predecessor, ua)),
                ("successor", opt(a.successor, ua)),
            ];
            let mut contains: Contains = Vec::new();
            match &a.kind {
                UserActionKind::EntryLevelSystemCall {
                    role: r,
                    signature,
                    inputs,
                    outputs,
                    priority,
                } => {
                    attrs.push(("priority", J::I(*priority)));
                    refs.push(("providedRole_EntryLevelSystemCall", opt(*r, role)));
                    refs.push((
                        "operationSignature__EntryLevelSystemCall",
                        opt(*signature, sig),
                    ));
                    contains.push((
                        "inputParameterUsages_EntryLevelSystemCall",
                        Self::uobjs(inputs),
                    ));
                    contains.push((
                        "outputParameterUsages_EntryLevelSystemCall",
                        Self::uobjs(outputs),
                    ));
                    self.usages(inputs);
                    self.usages(outputs);
                }
                UserActionKind::Delay { time } => {
                    contains.push(("timeSpecification_Delay", Self::rvs(time)));
                    self.rv(time);
                }
                UserActionKind::Branch { transitions } => {
                    contains.push((
                        "branchTransitions_Branch",
                        transitions.iter().map(|t| t.obj).collect(),
                    ));
                    for t in transitions {
                        self.line(
                            t.obj,
                            "usagemodel:BranchTransition",
                            "",
                            vec![("branchProbability", J::D(t.probability))],
                            vec![],
                            vec![(
                                "branchedBehaviour_BranchTransition",
                                t.behaviour.map(sb).into_iter().collect(),
                            )],
                        );
                    }
                }
                UserActionKind::Loop { iterations, body } => {
                    contains.push(("loopIteration_Loop", Self::rvs(iterations)));
                    contains.push(("bodyBehaviour_Loop", body.map(sb).into_iter().collect()));
                    self.rv(iterations);
                }
                UserActionKind::Start | UserActionKind::Stop => {}
            }
            self.line(a.obj, &ty, &a.id, attrs, refs, contains);
        }
        for r in m.monitor_repositories.iter() {
            self.line(
                r.obj,
                "monitorrepository:MonitorRepository",
                &r.id,
                vec![("entityName", J::S(Some(&r.name)))],
                vec![],
                vec![("monitors", r.monitors.iter().map(|x| mon(*x)).collect())],
            );
        }
        for mo in m.monitors.iter() {
            self.line(
                mo.obj,
                "monitorrepository:Monitor",
                &mo.id,
                vec![
                    ("entityName", J::S(Some(&mo.name))),
                    ("activated", J::B(mo.activated)),
                ],
                vec![("measuringPoint", opt(mo.measuring_point, mp))],
                vec![(
                    "measurementSpecifications",
                    mo.specifications.iter().map(|s| s.obj).collect(),
                )],
            );
            for s in &mo.specifications {
                let pt = self.g().get_ref(
                    s.obj,
                    crate::meta::feat::MeasurementSpecification_processingType,
                );
                self.line(
                    s.obj,
                    "monitorrepository:MeasurementSpecification",
                    &s.id,
                    vec![("triggersSelfAdaptations", J::B(s.triggers_self_adaptations))],
                    vec![("metricDescription", opt(s.metric, met))],
                    vec![("processingType", pt.into_iter().collect())],
                );
                if let Some(p) = pt {
                    let ty = self.g()[p].class.qualified_name();
                    let id = self.g().id(p).unwrap_or("").to_string();
                    let attrs: Attrs = match &s.processing {
                        ProcessingType::TimeDriven {
                            window_length,
                            window_increment,
                        }
                        | ProcessingType::TimeDrivenAggregation {
                            window_length,
                            window_increment,
                            ..
                        } => vec![
                            ("windowLength", J::D(*window_length)),
                            ("windowIncrement", J::D(*window_increment)),
                        ],
                        ProcessingType::FixedSizeAggregation {
                            frequency,
                            number_of_measurements,
                            ..
                        } => vec![
                            ("frequency", J::I(*frequency)),
                            ("numberOfMeasurements", J::I(*number_of_measurements)),
                        ],
                        ProcessingType::VariableSizeAggregation {
                            frequency,
                            retrospection_length,
                            ..
                        } => vec![
                            ("frequency", J::I(*frequency)),
                            ("retrospectionLength", J::D(*retrospection_length)),
                        ],
                        _ => vec![],
                    };
                    self.line(p, &ty, &id, attrs, vec![], vec![]);
                }
            }
        }
        for p in m.measuring_points.iter() {
            let ty = self.g()[p.obj].class.qualified_name();
            let mut attrs: Attrs = vec![];
            let refs: Refs = match &p.kind {
                MeasuringPointKind::AssemblyOperation {
                    assembly,
                    role: r,
                    signature,
                } => vec![
                    ("role", opt(*r, role)),
                    ("operationSignature", opt(*signature, sig)),
                    ("assembly", opt(*assembly, ac)),
                ],
                MeasuringPointKind::AssemblyPassiveResource {
                    assembly,
                    passive_resource,
                } => vec![
                    ("assembly", opt(*assembly, ac)),
                    ("passiveResource", opt(*passive_resource, pres)),
                ],
                MeasuringPointKind::ActiveResource {
                    resource,
                    replica_id,
                } => {
                    attrs.push(("replicaID", J::I(*replica_id)));
                    vec![("activeResource", opt(*resource, proc))]
                }
                MeasuringPointKind::SystemOperation {
                    system,
                    role: r,
                    signature,
                } => vec![
                    ("role", opt(*r, role)),
                    ("operationSignature", opt(*signature, sig)),
                    ("system", opt(*system, sys)),
                ],
                MeasuringPointKind::SubSystemOperation {
                    subsystem,
                    role: r,
                    signature,
                } => vec![
                    ("subsystem", opt(*subsystem, comp)),
                    ("role", opt(*r, role)),
                    ("operationSignature", opt(*signature, sig)),
                ],
                MeasuringPointKind::LinkingResource { resource } => {
                    vec![("linkingResource", opt(*resource, link))]
                }
                MeasuringPointKind::UsageScenario { scenario } => {
                    vec![("usageScenario", opt(*scenario, scen))]
                }
                MeasuringPointKind::EntryLevelSystemCall { call } => {
                    vec![("entryLevelSystemCall", opt(*call, ua))]
                }
                MeasuringPointKind::ExternalCallAction { action } => {
                    vec![("externalCall", opt(*action, act))]
                }
                MeasuringPointKind::ResourceEnvironment { environment } => {
                    vec![("resourceEnvironment", opt(*environment, env))]
                }
                MeasuringPointKind::ResourceContainer { container } => {
                    vec![("resourceContainer", opt(*container, cont))]
                }
                MeasuringPointKind::Other {
                    class,
                    measuring_point,
                } => {
                    if *class != "ReconfigurationMeasuringPoint" || measuring_point.is_some() {
                        attrs.push(("measuringPoint", J::S(measuring_point.as_deref())));
                    }
                    vec![]
                }
            };
            self.line(p.obj, &ty, "", attrs, refs, vec![]);
        }
        for x in m.metrics.iter() {
            let ty = self.g()[x.obj].class.qualified_name();
            let mut attrs: Attrs = vec![
                ("name", J::S(Some(&x.name))),
                ("textualDescription", J::S(x.textual_description.as_deref())),
            ];
            let mut refs: Refs = vec![];
            match &x.kind {
                MetricKind::Base {
                    numerical,
                    capture_type,
                    data_type,
                    scale,
                    scope_of_validity,
                    default_unit,
                } => {
                    attrs.push(("captureType", J::E(capture_type)));
                    attrs.push(("dataType", J::E(data_type)));
                    attrs.push(("scale", J::E(scale)));
                    attrs.push(("scopeOfValidity", J::E(scope_of_validity)));
                    if *numerical {
                        attrs.push(("defaultUnit", J::S(default_unit.as_deref())));
                    }
                }
                MetricKind::Set { subsumed } => refs.push(("subsumedMetrics", many(subsumed, met))),
                MetricKind::Other(_) => {}
            }
            self.line(x.obj, &ty, &x.id, attrs, refs, vec![]);
        }
    }
}

/// Dump of the typed model in canonical form: one line per typed element (and per inline value
/// object) with the EMF feature names the typed fields were read from. `contains` lists child
/// paths; `res` is the resource label of [`resource_labels`]. Used to check the typed layer
/// against the EMF golden files.
pub fn typed_dump(m: &Model) -> String {
    let mut w = W {
        m,
        labels: resource_labels(&m.graph),
        out: String::new(),
    };
    w.run();
    w.out
}

#[cfg(test)]
mod tests {
    use super::java_double_to_string as j;
    #[test]
    fn java_doubles() {
        assert_eq!(j(1.0), "1.0");
        assert_eq!(j(0.001), "0.001");
        assert_eq!(j(0.0001), "1.0E-4");
        assert_eq!(j(1e7), "1.0E7");
        assert_eq!(j(9999999.0), "9999999.0");
        assert_eq!(j(123.456), "123.456");
        assert_eq!(j(-2.5e-10), "-2.5E-10");
        assert_eq!(j(1.5e300), "1.5E300");
        assert_eq!(j(0.1), "0.1");
        assert_eq!(j(100.0), "100.0");
    }
}
