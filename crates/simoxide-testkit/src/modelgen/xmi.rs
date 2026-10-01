//! XMI writer for [`GenModel`]: `.repository`, `.system`, `.resourceenvironment`, `.allocation`,
//! `.usagemodel`, `.measuringpoint`, `.monitorrepository` (refsim default monitors, mirroring
//! `reference/src/refsim/corpus/Monitors.java`) and `run.json`. Layout follows the EMF output of the
//! reference's `PcmBuilder` (relative hrefs, same-document references as IDREF attributes).

use std::fmt::Write as _;
use std::path::Path;

use super::model::*;

const PRIM_INT: &str = "pathmap://PCM_MODELS/PrimitiveTypes.repository#//@dataTypes__Repository.0";
const RT: &str = "pathmap://PCM_MODELS/Palladio.resourcetype#";
const CPU_IF: &str = "_tw_Q8E5CEeCUKeckjJ_n-w";
const HDD_IF: &str = "_xXv8QE5CEeCUKeckjJ_n-w";
const UTILIZATION: &str = "_QIb6cikUEeSuf8LV7cHLgA";
const LAN: &str = "_o3sScH2AEdyH8uerKnHYug";
const METRICS: &str = "pathmap://METRIC_SPEC_MODELS/models/commonMetrics.metricspec#";
const RESPONSE_TIME: &str = "_6rYmYs7nEeOX_4BzImuHbA";
const STATE_ACTIVE: &str = "_paDhIs7qEeOX_4BzImuHbA";
const RESOURCE_DEMAND: &str = "_eg_F0s7qEeOX_4BzImuHbA";
const WAITING_TIME: &str = "_QWjAYs7qEeOX_4BzImuHbA";
const HOLDING_TIME: &str = "_zETOUs7pEeOX_4BzImuHbA";
const STATE_PASSIVE: &str = "_x0-pks7rEeOX_4BzImuHbA";
const RECONFIGURATION_TIME: &str = "_VYg6MujFEeSB6OBq2SKZxQ";
const NUMBER_OF_RESOURCE_CONTAINERS: &str = "_e7x3gq-eEeSgL6DrxYuwZg";

/// XML attribute escaping.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' => o.push_str("&#xA;"),
            _ => o.push(c),
        }
    }
    o
}

struct X {
    s: String,
}

impl X {
    fn new() -> X {
        X {
            s: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"),
        }
    }
    fn ind(&mut self, d: usize) {
        for _ in 0..d {
            self.s.push_str("  ");
        }
    }
    /// `<tag a="v" ...` + (`/>` if `close`) + newline.
    fn tag(&mut self, d: usize, tag: &str, attrs: &[(&str, &str)], close: bool) {
        self.ind(d);
        self.s.push('<');
        self.s.push_str(tag);
        for (k, v) in attrs {
            let _ = write!(self.s, " {k}=\"{}\"", esc(v));
        }
        self.s.push_str(if close { "/>\n" } else { ">\n" });
    }
    fn end(&mut self, d: usize, tag: &str) {
        self.ind(d);
        let _ = writeln!(self.s, "</{tag}>");
    }
    fn spec(&mut self, d: usize, tag: &str, spec: &str) {
        self.tag(d, tag, &[("specification", spec)], true);
    }
    fn href(&mut self, d: usize, tag: &str, xsi: Option<&str>, href: &str) {
        match xsi {
            Some(t) => self.tag(d, tag, &[("xsi:type", t), ("href", href)], true),
            None => self.tag(d, tag, &[("href", href)], true),
        }
    }
}

fn var_usage(x: &mut X, d: usize, tag: &str, u: &VarUsage) {
    x.tag(d, tag, &[], false);
    for (c, spec) in &u.chars {
        x.tag(
            d + 1,
            "variableCharacterisation_VariableUsage",
            &[("type", c.name())],
            false,
        );
        x.spec(d + 2, "specification_VariableCharacterisation", spec);
        x.end(d + 1, "variableCharacterisation_VariableUsage");
    }
    let parts: Vec<&str> = u.name.split('.').collect();
    named_ref(x, d + 1, "namedReference__VariableUsage", &parts);
    x.end(d, tag);
}

fn named_ref(x: &mut X, d: usize, tag: &str, parts: &[&str]) {
    if parts.len() == 1 {
        x.tag(
            d,
            tag,
            &[
                ("xsi:type", "stoex:VariableReference"),
                ("referenceName", parts[0]),
            ],
            true,
        );
    } else {
        x.tag(
            d,
            tag,
            &[
                ("xsi:type", "stoex:NamespaceReference"),
                ("referenceName", parts[0]),
            ],
            false,
        );
        named_ref(x, d + 1, "innerReference_NamespaceReference", &parts[1..]);
        x.end(d, tag);
    }
}

// ---------------------------------------------------------------------- repository

fn behaviour(x: &mut X, d: usize, tag: &str, b: &Behaviour, extra: &[(&str, &str)]) {
    let mut attrs = vec![("id", b.id.as_str())];
    attrs.extend_from_slice(extra);
    x.tag(d, tag, &attrs, false);
    steps(x, d + 1, b);
    x.end(d, tag);
}

fn steps(x: &mut X, d: usize, b: &Behaviour) {
    let ids: Vec<&str> = std::iter::once(b.start_id.as_str())
        .chain(b.actions.iter().map(|a| a.id()))
        .chain(std::iter::once(b.stop_id.as_str()))
        .collect();
    x.tag(
        d,
        "steps_Behaviour",
        &[
            ("xsi:type", "seff:StartAction"),
            ("id", &b.start_id),
            ("entityName", "start"),
            ("successor_AbstractAction", ids[1]),
        ],
        true,
    );
    for (i, a) in b.actions.iter().enumerate() {
        action(x, d, a, ids[i], ids[i + 2]);
    }
    x.tag(
        d,
        "steps_Behaviour",
        &[
            ("xsi:type", "seff:StopAction"),
            ("id", &b.stop_id),
            ("entityName", "stop"),
            ("predecessor_AbstractAction", ids[ids.len() - 2]),
        ],
        true,
    );
}

fn action(x: &mut X, d: usize, a: &Action, pred: &str, succ: &str) {
    const T: &str = "steps_Behaviour";
    let base = |ty: &'static str, id: &str, name: &str| -> Vec<(String, String)> {
        vec![
            ("xsi:type".into(), ty.into()),
            ("id".into(), id.into()),
            ("entityName".into(), name.into()),
            ("predecessor_AbstractAction".into(), pred.into()),
            ("successor_AbstractAction".into(), succ.into()),
        ]
    };
    let open = |x: &mut X, attrs: Vec<(String, String)>, close: bool| {
        let v: Vec<(&str, &str)> = attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        x.tag(d, T, &v, close);
    };
    match a {
        Action::Internal {
            id,
            name,
            demands,
            infra,
            rescalls,
        } => {
            open(x, base("seff:InternalAction", id, name), false);
            for (t, spec) in demands {
                x.tag(d + 1, "resourceDemand_Action", &[], false);
                x.spec(d + 2, "specification_ParametericResourceDemand", spec);
                x.href(
                    d + 2,
                    "requiredResource_ParametricResourceDemand",
                    None,
                    &format!("{RT}{}", t.type_id()),
                );
                x.end(d + 1, "resourceDemand_Action");
            }
            for ic in infra {
                x.tag(
                    d + 1,
                    "infrastructureCall__Action",
                    &[
                        ("id", &ic.id),
                        ("signature__InfrastructureCall", &ic.sig),
                        ("requiredRole__InfrastructureCall", &ic.role),
                    ],
                    false,
                );
                x.spec(d + 2, "numberOfCalls__InfrastructureCall", &ic.count);
                for u in &ic.inputs {
                    var_usage(x, d + 2, "inputVariableUsages__CallAction", u);
                }
                x.end(d + 1, "infrastructureCall__Action");
            }
            for rc in rescalls {
                x.tag(
                    d + 1,
                    "resourceCall__Action",
                    &[
                        ("id", &rc.id),
                        ("resourceRequiredRole__ResourceCall", &rc.role),
                    ],
                    false,
                );
                x.href(
                    d + 2,
                    "signature__ResourceCall",
                    None,
                    &format!("{RT}{}", rc.sig.id()),
                );
                x.spec(d + 2, "numberOfCalls__ResourceCall", &rc.count);
                x.end(d + 1, "resourceCall__Action");
            }
            x.end(d, T);
        }
        Action::External {
            id,
            name,
            role,
            sig,
            inputs,
            returns,
        } => {
            let mut at = base("seff:ExternalCallAction", id, name);
            at.push(("calledService_ExternalService".into(), sig.clone()));
            at.push(("role_ExternalService".into(), role.clone()));
            let leaf = inputs.is_empty() && returns.is_empty();
            open(x, at, leaf);
            if !leaf {
                for u in inputs {
                    var_usage(x, d + 1, "inputVariableUsages__CallAction", u);
                }
                for u in returns {
                    var_usage(x, d + 1, "returnVariableUsage__CallReturnAction", u);
                }
                x.end(d, T);
            }
        }
        Action::ProbBranch { id, name, trans } => {
            open(x, base("seff:BranchAction", id, name), false);
            for (k, (tid, p, b)) in trans.iter().enumerate() {
                let n = format!("{name}_{k}");
                x.tag(
                    d + 1,
                    "branches_Branch",
                    &[
                        ("xsi:type", "seff:ProbabilisticBranchTransition"),
                        ("id", tid),
                        ("entityName", &n),
                        ("branchProbability", p),
                    ],
                    false,
                );
                behaviour(x, d + 2, "branchBehaviour_BranchTransition", b, &[]);
                x.end(d + 1, "branches_Branch");
            }
            x.end(d, T);
        }
        Action::GuardedBranch { id, name, trans } => {
            open(x, base("seff:BranchAction", id, name), false);
            for (k, (tid, cond, b)) in trans.iter().enumerate() {
                let n = format!("{name}_{k}");
                x.tag(
                    d + 1,
                    "branches_Branch",
                    &[
                        ("xsi:type", "seff:GuardedBranchTransition"),
                        ("id", tid),
                        ("entityName", &n),
                    ],
                    false,
                );
                behaviour(x, d + 2, "branchBehaviour_BranchTransition", b, &[]);
                x.spec(d + 2, "branchCondition_GuardedBranchTransition", cond);
                x.end(d + 1, "branches_Branch");
            }
            x.end(d, T);
        }
        Action::Loop {
            id,
            name,
            count,
            body,
        } => {
            open(x, base("seff:LoopAction", id, name), false);
            behaviour(x, d + 1, "bodyBehaviour_Loop", body, &[]);
            x.spec(d + 1, "iterationCount_LoopAction", count);
            x.end(d, T);
        }
        Action::Iterate {
            id,
            name,
            param_path,
            body,
        } => {
            let mut at = base("seff:CollectionIteratorAction", id, name);
            at.push((
                "parameter_CollectionIteratorAction".into(),
                param_path.clone(),
            ));
            open(x, at, false);
            behaviour(x, d + 1, "bodyBehaviour_Loop", body, &[]);
            x.end(d, T);
        }
        Action::Fork {
            id,
            name,
            asyncs,
            sync,
        } => {
            open(x, base("seff:ForkAction", id, name), false);
            for b in asyncs {
                behaviour(x, d + 1, "asynchronousForkedBehaviours_ForkAction", b, &[]);
            }
            if let Some((sp, bs)) = sync {
                x.tag(
                    d + 1,
                    "synchronisingBehaviours_ForkAction",
                    &[("id", sp)],
                    false,
                );
                for b in bs {
                    behaviour(
                        x,
                        d + 2,
                        "synchronousForkedBehaviours_SynchronisationPoint",
                        b,
                        &[],
                    );
                }
                x.end(d + 1, "synchronisingBehaviours_ForkAction");
            }
            x.end(d, T);
        }
        Action::Acquire { id, name, pr } => {
            let mut at = base("seff:AcquireAction", id, name);
            at.push(("passiveresource_AcquireAction".into(), pr.clone()));
            open(x, at, true);
        }
        Action::Release { id, name, pr } => {
            let mut at = base("seff:ReleaseAction", id, name);
            at.push(("passiveResource_ReleaseAction".into(), pr.clone()));
            open(x, at, true);
        }
        Action::SetVar { id, name, usages } => {
            open(x, base("seff:SetVariableAction", id, name), false);
            for u in usages {
                var_usage(x, d + 1, "localVariableUsages_SetVariableAction", u);
            }
            x.end(d, T);
        }
    }
}

fn role_tags(m: &GenModel, x: &mut X, d: usize, provides: &[Role], requires: &[Role]) {
    for r in provides {
        let i = &m.interfaces[r.iface];
        if i.infra {
            x.tag(
                d,
                "providedRoles_InterfaceProvidingEntity",
                &[
                    ("xsi:type", "repository:InfrastructureProvidedRole"),
                    ("id", &r.id),
                    ("entityName", &r.name),
                    ("providedInterface__InfrastructureProvidedRole", &i.id),
                ],
                true,
            );
        } else {
            x.tag(
                d,
                "providedRoles_InterfaceProvidingEntity",
                &[
                    ("xsi:type", "repository:OperationProvidedRole"),
                    ("id", &r.id),
                    ("entityName", &r.name),
                    ("providedInterface__OperationProvidedRole", &i.id),
                ],
                true,
            );
        }
    }
    for r in requires {
        let i = &m.interfaces[r.iface];
        if i.infra {
            x.tag(
                d,
                "requiredRoles_InterfaceRequiringEntity",
                &[
                    ("xsi:type", "repository:InfrastructureRequiredRole"),
                    ("id", &r.id),
                    ("entityName", &r.name),
                    ("requiredInterface__InfrastructureRequiredRole", &i.id),
                ],
                true,
            );
        } else {
            x.tag(
                d,
                "requiredRoles_InterfaceRequiringEntity",
                &[
                    ("xsi:type", "repository:OperationRequiredRole"),
                    ("id", &r.id),
                    ("entityName", &r.name),
                    ("requiredInterface__OperationRequiredRole", &i.id),
                ],
                true,
            );
        }
    }
}

fn conn_tag(x: &mut X, d: usize, c: &Conn, repo: Option<&str>) {
    // repo = Some(file) when written in the system (roles of components are in the repository)
    const T: &str = "connectors__ComposedStructure";
    match c {
        Conn::Assembly {
            id,
            req_ac,
            req_role,
            prov_ac,
            prov_role,
        } => {
            let mut a = vec![
                ("xsi:type", "composition:AssemblyConnector"),
                ("id", id.as_str()),
                ("entityName", "Connector"),
                (
                    "requiringAssemblyContext_AssemblyConnector",
                    req_ac.as_str(),
                ),
                (
                    "providingAssemblyContext_AssemblyConnector",
                    prov_ac.as_str(),
                ),
            ];
            match repo {
                None => {
                    a.push(("providedRole_AssemblyConnector", prov_role));
                    a.push(("requiredRole_AssemblyConnector", req_role));
                    x.tag(d, T, &a, true);
                }
                Some(r) => {
                    x.tag(d, T, &a, false);
                    x.href(
                        d + 1,
                        "providedRole_AssemblyConnector",
                        None,
                        &format!("{r}#{prov_role}"),
                    );
                    x.href(
                        d + 1,
                        "requiredRole_AssemblyConnector",
                        None,
                        &format!("{r}#{req_role}"),
                    );
                    x.end(d, T);
                }
            }
        }
        Conn::Infra {
            id,
            req_ac,
            req_role,
            prov_ac,
            prov_role,
        } => {
            let r = repo.expect("infrastructure connectors only at system level");
            x.tag(
                d,
                T,
                &[
                    ("xsi:type", "composition:AssemblyInfrastructureConnector"),
                    ("id", id),
                    ("entityName", "InfraConnector"),
                    (
                        "providingAssemblyContext__AssemblyInfrastructureConnector",
                        prov_ac,
                    ),
                    (
                        "requiringAssemblyContext__AssemblyInfrastructureConnector",
                        req_ac,
                    ),
                ],
                false,
            );
            x.href(
                d + 1,
                "providedRole__AssemblyInfrastructureConnector",
                None,
                &format!("{r}#{prov_role}"),
            );
            x.href(
                d + 1,
                "requiredRole__AssemblyInfrastructureConnector",
                None,
                &format!("{r}#{req_role}"),
            );
            x.end(d, T);
        }
        Conn::ProvDeleg {
            id,
            outer_role,
            ac,
            inner_role,
        } => {
            let mut a = vec![
                ("xsi:type", "composition:ProvidedDelegationConnector"),
                ("id", id.as_str()),
                ("entityName", "ProvDelegation"),
                (
                    "outerProvidedRole_ProvidedDelegationConnector",
                    outer_role.as_str(),
                ),
                ("assemblyContext_ProvidedDelegationConnector", ac.as_str()),
            ];
            match repo {
                None => {
                    a.push(("innerProvidedRole_ProvidedDelegationConnector", inner_role));
                    x.tag(d, T, &a, true);
                }
                Some(r) => {
                    x.tag(d, T, &a, false);
                    x.href(
                        d + 1,
                        "innerProvidedRole_ProvidedDelegationConnector",
                        None,
                        &format!("{r}#{inner_role}"),
                    );
                    x.end(d, T);
                }
            }
        }
        Conn::ReqDeleg {
            id,
            inner_role,
            ac,
            outer_role,
        } => {
            assert!(repo.is_none(), "required delegation only inside composites");
            x.tag(
                d,
                T,
                &[
                    ("xsi:type", "composition:RequiredDelegationConnector"),
                    ("id", id),
                    ("entityName", "ReqDelegation"),
                    ("innerRequiredRole_RequiredDelegationConnector", inner_role),
                    ("outerRequiredRole_RequiredDelegationConnector", outer_role),
                    ("assemblyContext_RequiredDelegationConnector", ac),
                ],
                true,
            );
        }
    }
}

fn repository(m: &GenModel) -> String {
    let n = &m.name;
    let mut x = X::new();
    let rid = format!("_{n}_repository1");
    x.s.push_str(&format!(
        "<repository:Repository xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:composition=\"http://palladiosimulator.org/PalladioComponentModel/Core/Composition/5.2\" \
         xmlns:repository=\"http://palladiosimulator.org/PalladioComponentModel/Repository/5.2\" \
         xmlns:seff=\"http://palladiosimulator.org/PalladioComponentModel/SEFF/5.2\" \
         xmlns:stoex=\"http://sdq.ipd.uka.de/StochasticExpressions/2.2\" id=\"{}\" entityName=\"{}\">\n",
        esc(&rid),
        esc(n)
    ));
    for c in &m.components {
        match c {
            Component::Basic(b) => {
                x.tag(
                    1,
                    "components__Repository",
                    &[
                        ("xsi:type", "repository:BasicComponent"),
                        ("id", &b.id),
                        ("entityName", &b.name),
                    ],
                    false,
                );
                role_tags(m, &mut x, 2, &b.provides, &b.requires);
                if let Some(r) = &b.rreq {
                    x.tag(
                        2,
                        "resourceRequiredRoles__ResourceInterfaceRequiringEntity",
                        &[("id", r), ("entityName", "CpuRequired")],
                        false,
                    );
                    x.href(
                        3,
                        "requiredResourceInterface__ResourceRequiredRole",
                        None,
                        &format!("{RT}{CPU_IF}"),
                    );
                    x.end(2, "resourceRequiredRoles__ResourceInterfaceRequiringEntity");
                }
                if let Some(r) = &b.rreq_hdd {
                    x.tag(
                        2,
                        "resourceRequiredRoles__ResourceInterfaceRequiringEntity",
                        &[("id", r), ("entityName", "HddRequired")],
                        false,
                    );
                    x.href(
                        3,
                        "requiredResourceInterface__ResourceRequiredRole",
                        None,
                        &format!("{RT}{HDD_IF}"),
                    );
                    x.end(2, "resourceRequiredRoles__ResourceInterfaceRequiringEntity");
                }
                for u in &b.comp_params {
                    var_usage(
                        &mut x,
                        2,
                        "componentParameterUsage_ImplementationComponentType",
                        u,
                    );
                }
                for s in &b.seffs {
                    x.tag(
                        2,
                        "serviceEffectSpecifications__BasicComponent",
                        &[
                            ("xsi:type", "seff:ResourceDemandingSEFF"),
                            ("id", &s.id),
                            ("describedService__SEFF", &s.sig),
                        ],
                        false,
                    );
                    steps(&mut x, 3, &s.body);
                    x.end(2, "serviceEffectSpecifications__BasicComponent");
                }
                for p in &b.passive {
                    x.tag(
                        2,
                        "passiveResource_BasicComponent",
                        &[("id", &p.id), ("entityName", &p.name)],
                        false,
                    );
                    x.spec(3, "capacity_PassiveResource", &p.capacity);
                    x.end(2, "passiveResource_BasicComponent");
                }
                x.end(1, "components__Repository");
            }
            Component::Composite(cc) => {
                x.tag(
                    1,
                    "components__Repository",
                    &[
                        ("xsi:type", "repository:CompositeComponent"),
                        ("id", &cc.id),
                        ("entityName", &cc.name),
                    ],
                    false,
                );
                for a in &cc.inner {
                    x.tag(
                        2,
                        "assemblyContexts__ComposedStructure",
                        &[
                            ("id", &a.id),
                            ("entityName", &a.name),
                            (
                                "encapsulatedComponent__AssemblyContext",
                                m.components[a.comp].id(),
                            ),
                        ],
                        true,
                    );
                }
                for c in &cc.conns {
                    conn_tag(&mut x, 2, c, None);
                }
                role_tags(m, &mut x, 2, &cc.provides, &cc.requires);
                x.end(1, "components__Repository");
            }
        }
    }
    for i in &m.interfaces {
        if i.infra {
            x.tag(
                1,
                "interfaces__Repository",
                &[
                    ("xsi:type", "repository:InfrastructureInterface"),
                    ("id", &i.id),
                    ("entityName", &i.name),
                ],
                false,
            );
            for s in &i.sigs {
                sig_tag(
                    &mut x,
                    2,
                    "infrastructureSignatures__InfrastructureInterface",
                    "parameters__InfrastructureSignature",
                    s,
                    m,
                );
            }
        } else {
            x.tag(
                1,
                "interfaces__Repository",
                &[
                    ("xsi:type", "repository:OperationInterface"),
                    ("id", &i.id),
                    ("entityName", &i.name),
                ],
                false,
            );
            for s in &i.sigs {
                sig_tag(
                    &mut x,
                    2,
                    "signatures__OperationInterface",
                    "parameters__OperationSignature",
                    s,
                    m,
                );
            }
        }
        x.end(1, "interfaces__Repository");
    }
    if let Some(dt) = &m.coll_type {
        x.tag(
            1,
            "dataTypes__Repository",
            &[
                ("xsi:type", "repository:CollectionDataType"),
                ("id", dt),
                ("entityName", "IntList"),
            ],
            false,
        );
        x.href(
            2,
            "innerType_CollectionDataType",
            Some("repository:PrimitiveDataType"),
            PRIM_INT,
        );
        x.end(1, "dataTypes__Repository");
    }
    x.s.push_str("</repository:Repository>\n");
    x.s
}

fn sig_tag(x: &mut X, d: usize, tag: &str, ptag: &str, s: &Signature, m: &GenModel) {
    let leaf = s.params.is_empty() && !s.returns;
    x.tag(d, tag, &[("id", &s.id), ("entityName", &s.name)], leaf);
    if leaf {
        return;
    }
    for p in &s.params {
        match p.kind {
            ParamKind::Coll => {
                let dt = m.coll_type.as_deref().unwrap_or("");
                x.tag(
                    d + 1,
                    ptag,
                    &[("dataType__Parameter", dt), ("parameterName", &p.name)],
                    true,
                );
            }
            _ => {
                x.tag(d + 1, ptag, &[("parameterName", &p.name)], false);
                x.href(
                    d + 2,
                    "dataType__Parameter",
                    Some("repository:PrimitiveDataType"),
                    PRIM_INT,
                );
                x.end(d + 1, ptag);
            }
        }
    }
    if s.returns {
        x.href(
            d + 1,
            "returnType__OperationSignature",
            Some("repository:PrimitiveDataType"),
            PRIM_INT,
        );
    }
    x.end(d, tag);
}

// ---------------------------------------------------------------------- system, env, allocation

fn system(m: &GenModel) -> String {
    let n = &m.name;
    let repo = format!("{n}.repository");
    let mut x = X::new();
    x.s.push_str(&format!(
        "<system:System xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:composition=\"http://palladiosimulator.org/PalladioComponentModel/Core/Composition/5.2\" \
         xmlns:repository=\"http://palladiosimulator.org/PalladioComponentModel/Repository/5.2\" \
         xmlns:stoex=\"http://sdq.ipd.uka.de/StochasticExpressions/2.2\" \
         xmlns:system=\"http://palladiosimulator.org/PalladioComponentModel/System/5.2\" id=\"{}\" entityName=\"{}\">\n",
        esc(&system_id(m)),
        esc(n)
    ));
    for a in &m.assemblies {
        x.tag(
            1,
            "assemblyContexts__ComposedStructure",
            &[("id", &a.id), ("entityName", &a.name)],
            false,
        );
        let c = &m.components[a.comp];
        let ty = match c {
            Component::Basic(_) => "repository:BasicComponent",
            Component::Composite(_) => "repository:CompositeComponent",
        };
        x.href(
            2,
            "encapsulatedComponent__AssemblyContext",
            Some(ty),
            &format!("{repo}#{}", c.id()),
        );
        for u in &a.config {
            var_usage(&mut x, 2, "configParameterUsages__AssemblyContext", u);
        }
        x.end(1, "assemblyContexts__ComposedStructure");
    }
    for c in &m.connectors {
        conn_tag(&mut x, 1, c, Some(&repo));
    }
    for r in &m.sys_roles {
        x.tag(
            1,
            "providedRoles_InterfaceProvidingEntity",
            &[
                ("xsi:type", "repository:OperationProvidedRole"),
                ("id", &r.id),
                ("entityName", &r.name),
            ],
            false,
        );
        x.href(
            2,
            "providedInterface__OperationProvidedRole",
            None,
            &format!("{repo}#{}", m.interfaces[r.iface].id),
        );
        x.end(1, "providedRoles_InterfaceProvidingEntity");
    }
    x.s.push_str("</system:System>\n");
    x.s
}

fn system_id(m: &GenModel) -> String {
    format!("_{}_system1", m.name)
}

fn resenv(m: &GenModel) -> String {
    let mut x = X::new();
    x.s.push_str(&format!(
        "<resourceenvironment:ResourceEnvironment xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:resourceenvironment=\"http://palladiosimulator.org/PalladioComponentModel/ResourceEnvironment/5.2\" \
         entityName=\"{}\">\n",
        esc(&m.name)
    ));
    for l in &m.links {
        let rcs: Vec<&str> = l
            .containers
            .iter()
            .map(|&c| m.containers[c].id.as_str())
            .collect();
        let rcs = rcs.join(" ");
        x.tag(
            1,
            "linkingResources__ResourceEnvironment",
            &[
                ("id", &l.id),
                ("entityName", &l.name),
                ("connectedResourceContainers_LinkingResource", &rcs),
            ],
            false,
        );
        x.tag(
            2,
            "communicationLinkResourceSpecifications_LinkingResource",
            &[("id", &l.spec_id)],
            false,
        );
        x.href(
            3,
            "communicationLinkResourceType_CommunicationLinkResourceSpecification",
            None,
            &format!("{RT}{LAN}"),
        );
        x.spec(
            3,
            "latency_CommunicationLinkResourceSpecification",
            &l.latency,
        );
        x.spec(
            3,
            "throughput_CommunicationLinkResourceSpecification",
            &l.throughput,
        );
        x.end(2, "communicationLinkResourceSpecifications_LinkingResource");
        x.end(1, "linkingResources__ResourceEnvironment");
    }
    for (ci, c) in m.containers.iter().enumerate() {
        x.tag(
            1,
            "resourceContainer_ResourceEnvironment",
            &[("id", &c.id), ("entityName", &c.name)],
            false,
        );
        container_resources(&mut x, 2, c);
        if let Some((p, nc, _)) = &m.nested
            && *p == ci
        {
            x.tag(
                2,
                "nestedResourceContainers__ResourceContainer",
                &[("id", &nc.id), ("entityName", &nc.name)],
                false,
            );
            container_resources(&mut x, 3, nc);
            x.end(2, "nestedResourceContainers__ResourceContainer");
        }
        x.end(1, "resourceContainer_ResourceEnvironment");
    }
    x.s.push_str("</resourceenvironment:ResourceEnvironment>\n");
    x.s
}

/// The processing resources of a container at indentation `d`.
fn container_resources(x: &mut X, d: usize, c: &Container) {
    for r in &c.res {
        let reps = r.replicas.to_string();
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if r.hdd_rates.is_some() {
            attrs.push((
                "xsi:type",
                "resourceenvironment:HDDProcessingResourceSpecification",
            ));
        }
        attrs.push(("id", &r.id));
        if r.replicas > 1 {
            attrs.push(("numberOfReplicas", &reps));
        }
        x.tag(
            d,
            "activeResourceSpecifications_ResourceContainer",
            &attrs,
            false,
        );
        x.href(
            d + 1,
            "schedulingPolicy",
            None,
            &format!("{RT}{}", r.sched.policy()),
        );
        x.href(
            d + 1,
            "activeResourceType_ActiveResourceSpecification",
            None,
            &format!("{RT}{}", r.ty.type_id()),
        );
        x.spec(
            d + 1,
            "processingRate_ProcessingResourceSpecification",
            &r.rate,
        );
        if let Some((read, write)) = &r.hdd_rates {
            x.spec(d + 1, "writeProcessingRate", write);
            x.spec(d + 1, "readProcessingRate", read);
        }
        x.end(d, "activeResourceSpecifications_ResourceContainer");
    }
}

fn allocation(m: &GenModel) -> String {
    let n = &m.name;
    let mut x = X::new();
    x.s.push_str(&format!(
        "<allocation:Allocation xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" \
         xmlns:allocation=\"http://palladiosimulator.org/PalladioComponentModel/Allocation/5.2\" \
         id=\"_{}_allocation1\" entityName=\"{}\">\n",
        esc(n),
        esc(n)
    ));
    x.href(
        1,
        "targetResourceEnvironment_Allocation",
        None,
        &format!("{n}.resourceenvironment#/"),
    );
    x.href(
        1,
        "system_Allocation",
        None,
        &format!("{n}.system#{}", system_id(m)),
    );
    for (k, (id, a, c)) in m.allocation.iter().enumerate() {
        let asm = &m.assemblies[*a];
        let container_id = match &m.nested {
            Some((_, nc, Some(i))) if *i == k => &nc.id,
            _ => &m.containers[*c].id,
        };
        let name = format!("Allocation_{}", asm.name);
        x.tag(
            1,
            "allocationContexts_Allocation",
            &[("id", id), ("entityName", &name)],
            false,
        );
        x.href(
            2,
            "resourceContainer_AllocationContext",
            None,
            &format!("{n}.resourceenvironment#{}", container_id),
        );
        x.href(
            2,
            "assemblyContext_AllocationContext",
            None,
            &format!("{n}.system#{}", asm.id),
        );
        x.end(1, "allocationContexts_Allocation");
    }
    x.s.push_str("</allocation:Allocation>\n");
    x.s
}

// ---------------------------------------------------------------------- usage model

fn ubehaviour(x: &mut X, d: usize, tag: &str, b: &UBehaviour, m: &GenModel) {
    x.tag(d, tag, &[("id", &b.id), ("entityName", "behaviour")], false);
    let ids: Vec<&str> = std::iter::once(b.start_id.as_str())
        .chain(b.actions.iter().map(|a| match a {
            UAction::Call { id, .. }
            | UAction::Delay { id, .. }
            | UAction::Branch { id, .. }
            | UAction::Loop { id, .. } => id.as_str(),
        }))
        .chain(std::iter::once(b.stop_id.as_str()))
        .collect();
    const T: &str = "actions_ScenarioBehaviour";
    x.tag(
        d + 1,
        T,
        &[
            ("xsi:type", "usagemodel:Start"),
            ("id", &b.start_id),
            ("entityName", "start"),
            ("successor", ids[1]),
        ],
        true,
    );
    for (i, a) in b.actions.iter().enumerate() {
        let (pred, succ) = (ids[i], ids[i + 2]);
        match a {
            UAction::Call {
                id,
                name,
                sys_role,
                sig,
                inputs,
            } => {
                x.tag(
                    d + 1,
                    T,
                    &[
                        ("xsi:type", "usagemodel:EntryLevelSystemCall"),
                        ("id", id),
                        ("entityName", name),
                        ("successor", succ),
                        ("predecessor", pred),
                    ],
                    false,
                );
                x.href(
                    d + 2,
                    "providedRole_EntryLevelSystemCall",
                    None,
                    &format!("{}.system#{sys_role}", m.name),
                );
                x.href(
                    d + 2,
                    "operationSignature__EntryLevelSystemCall",
                    None,
                    &format!("{}.repository#{sig}", m.name),
                );
                for u in inputs {
                    var_usage(x, d + 2, "inputParameterUsages_EntryLevelSystemCall", u);
                }
                x.end(d + 1, T);
            }
            UAction::Delay { id, name, spec } => {
                x.tag(
                    d + 1,
                    T,
                    &[
                        ("xsi:type", "usagemodel:Delay"),
                        ("id", id),
                        ("entityName", name),
                        ("successor", succ),
                        ("predecessor", pred),
                    ],
                    false,
                );
                x.spec(d + 2, "timeSpecification_Delay", spec);
                x.end(d + 1, T);
            }
            UAction::Branch { id, name, trans } => {
                x.tag(
                    d + 1,
                    T,
                    &[
                        ("xsi:type", "usagemodel:Branch"),
                        ("id", id),
                        ("entityName", name),
                        ("successor", succ),
                        ("predecessor", pred),
                    ],
                    false,
                );
                for (p, b) in trans {
                    x.tag(
                        d + 2,
                        "branchTransitions_Branch",
                        &[("branchProbability", p)],
                        false,
                    );
                    ubehaviour(x, d + 3, "branchedBehaviour_BranchTransition", b, m);
                    x.end(d + 2, "branchTransitions_Branch");
                }
                x.end(d + 1, T);
            }
            UAction::Loop {
                id,
                name,
                count,
                body,
            } => {
                x.tag(
                    d + 1,
                    T,
                    &[
                        ("xsi:type", "usagemodel:Loop"),
                        ("id", id),
                        ("entityName", name),
                        ("successor", succ),
                        ("predecessor", pred),
                    ],
                    false,
                );
                x.spec(d + 2, "loopIteration_Loop", count);
                ubehaviour(x, d + 2, "bodyBehaviour_Loop", body, m);
                x.end(d + 1, T);
            }
        }
    }
    x.tag(
        d + 1,
        T,
        &[
            ("xsi:type", "usagemodel:Stop"),
            ("id", &b.stop_id),
            ("entityName", "stop"),
            ("predecessor", ids[ids.len() - 2]),
        ],
        true,
    );
    x.end(d, tag);
}

fn usagemodel(m: &GenModel) -> String {
    let mut x = X::new();
    x.s.push_str(
        "<usagemodel:UsageModel xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:stoex=\"http://sdq.ipd.uka.de/StochasticExpressions/2.2\" \
         xmlns:usagemodel=\"http://palladiosimulator.org/PalladioComponentModel/UsageModel/5.2\">\n",
    );
    for s in &m.scenarios {
        x.tag(
            1,
            "usageScenario_UsageModel",
            &[("id", &s.id), ("entityName", &s.name)],
            false,
        );
        ubehaviour(&mut x, 2, "scenarioBehaviour_UsageScenario", &s.body, m);
        match &s.workload {
            Workload::Open { interarrival } => {
                x.tag(
                    2,
                    "workload_UsageScenario",
                    &[("xsi:type", "usagemodel:OpenWorkload")],
                    false,
                );
                x.spec(3, "interArrivalTime_OpenWorkload", interarrival);
            }
            Workload::Closed { population, think } => {
                let p = population.to_string();
                x.tag(
                    2,
                    "workload_UsageScenario",
                    &[
                        ("xsi:type", "usagemodel:ClosedWorkload"),
                        ("population", &p),
                    ],
                    false,
                );
                x.spec(3, "thinkTime_ClosedWorkload", think);
            }
        }
        x.end(2, "workload_UsageScenario");
        x.end(1, "usageScenario_UsageModel");
    }
    x.s.push_str("</usagemodel:UsageModel>\n");
    x.s
}

// ---------------------------------------------------------------------- monitors

/// A measuring point: (xsi type, string representation, children (tag, xsi type, href), replica).
struct Mp {
    ty: &'static str,
    label: String,
    refs: Vec<(&'static str, Option<&'static str>, String)>,
    replica: Option<u32>,
    /// Further attributes (e.g. of a `ResourceURIMeasuringPoint`).
    attrs: Vec<(&'static str, String)>,
}

fn external_calls<'a>(b: &'a Behaviour, out: &mut Vec<&'a Action>) {
    for a in &b.actions {
        // eAllContents order: the action itself, then its contents
        if matches!(a, Action::External { .. }) {
            out.push(a);
        }
        match a {
            Action::ProbBranch { trans, .. } | Action::GuardedBranch { trans, .. } => {
                for (_, _, b) in trans {
                    external_calls(b, out);
                }
            }
            Action::Loop { body, .. } | Action::Iterate { body, .. } => external_calls(body, out),
            Action::Fork { asyncs, sync, .. } => {
                for b in asyncs {
                    external_calls(b, out);
                }
                if let Some((_, bs)) = sync {
                    for b in bs {
                        external_calls(b, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn ucalls<'a>(b: &'a UBehaviour, out: &mut Vec<(&'a str, &'a str)>) {
    for a in &b.actions {
        match a {
            UAction::Call { id, name, .. } => out.push((id, name)),
            UAction::Branch { trans, .. } => {
                for (_, b) in trans {
                    ucalls(b, out);
                }
            }
            UAction::Loop { body, .. } => ucalls(body, out),
            UAction::Delay { .. } => {}
        }
    }
}

/// Components in system order (recursively through composites) and passive-resource owners.
fn collect(
    m: &GenModel,
    asms: &[Assembly],
    comps: &mut Vec<usize>,
    passive: &mut Vec<(String, String, usize)>,
) {
    for a in asms {
        if !comps.contains(&a.comp) {
            comps.push(a.comp);
        }
        match &m.components[a.comp] {
            Component::Basic(b) => {
                if !b.passive.is_empty() {
                    passive.push((a.id.clone(), a.name.clone(), a.comp));
                }
            }
            Component::Composite(c) => collect(m, &c.inner, comps, passive),
        }
    }
}

fn monitors(m: &GenModel) -> (String, String) {
    let n = &m.name;
    let (repo, sys, usage, env) = (
        format!("{n}.repository"),
        format!("{n}.system"),
        format!("{n}.usagemodel"),
        format!("{n}.resourceenvironment"),
    );
    let mut mps: Vec<(Mp, String, Vec<&'static str>)> = Vec::new();
    // extra sliding-window specifications per measuring point index: (metric, processing type,
    // window length, increment)
    let mut extras: std::collections::BTreeMap<usize, Vec<(&'static str, &'static str, f64, f64)>> =
        Default::default();
    // PRM-only aggregations replacing a scenario's FeedThrough: measuring point index -> spec
    let mut aggs: std::collections::BTreeMap<usize, &Aggregation> = Default::default();
    for (si, s) in m.scenarios.iter().enumerate() {
        if let Some(a) = m.aggregations.iter().find(|a| a.scenario == si) {
            aggs.insert(mps.len(), a);
        }
        for w in &m.windows {
            if w.target == WindowTarget::Scenario(si) {
                extras.entry(mps.len()).or_default().push((
                    RESPONSE_TIME,
                    "monitorrepository:TimeDrivenAggregation",
                    w.len,
                    w.inc,
                ));
            }
        }
        mps.push((
            Mp {
                ty: "pcmmeasuringpoint:UsageScenarioMeasuringPoint",
                label: format!("Usage Scenario: {}", s.name),
                refs: vec![("usageScenario", None, format!("{usage}#{}", s.id))],
                replica: None,
                attrs: vec![],
            },
            format!("RT scenario {}", s.name),
            vec![RESPONSE_TIME],
        ));
    }
    let mut calls = Vec::new();
    for s in &m.scenarios {
        ucalls(&s.body, &mut calls);
    }
    for (id, name) in calls {
        mps.push((
            Mp {
                ty: "pcmmeasuringpoint:EntryLevelSystemCallMeasuringPoint",
                label: format!("SystemCall {name} [{id}]"),
                refs: vec![("entryLevelSystemCall", None, format!("{usage}#{id}"))],
                replica: None,
                attrs: vec![],
            },
            format!("RT call {name}"),
            vec![RESPONSE_TIME],
        ));
    }
    for r in &m.sys_roles {
        for s in &m.interfaces[r.iface].sigs {
            mps.push((
                Mp {
                    ty: "pcmmeasuringpoint:SystemOperationMeasuringPoint",
                    label: format!("{}.{}.{}", m.name, r.name, s.name),
                    refs: vec![
                        (
                            "role",
                            Some("repository:OperationProvidedRole"),
                            format!("{sys}#{}", r.id),
                        ),
                        ("operationSignature", None, format!("{repo}#{}", s.id)),
                        ("system", None, format!("{sys}#{}", system_id(m))),
                    ],
                    replica: None,
                    attrs: vec![],
                },
                format!("RT system op {}", s.name),
                vec![RESPONSE_TIME],
            ));
        }
    }
    for (ac, in_system, role, sig) in &m.asm_op_monitors {
        let file = if *in_system { &sys } else { &repo };
        mps.push((
            Mp {
                ty: "pcmmeasuringpoint:AssemblyOperationMeasuringPoint",
                label: format!("AssemblyOperation {ac} {role} {sig}"),
                refs: vec![
                    ("assembly", None, format!("{file}#{ac}")),
                    (
                        "role",
                        Some("repository:OperationProvidedRole"),
                        format!("{repo}#{role}"),
                    ),
                    ("operationSignature", None, format!("{repo}#{sig}")),
                ],
                replica: None,
                attrs: vec![],
            },
            format!("RT assembly op {sig}"),
            vec![RESPONSE_TIME],
        ));
    }
    let mut comps = Vec::new();
    let mut passive = Vec::new();
    collect(m, &m.assemblies, &mut comps, &mut passive);
    let mut ext: Vec<&Action> = Vec::new();
    for &c in &comps {
        if let Component::Basic(b) = &m.components[c] {
            for s in &b.seffs {
                external_calls(&s.body, &mut ext);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for a in ext {
        if let Action::External { id, name, .. } = a
            && seen.insert(id.clone())
        {
            mps.push((
                Mp {
                    ty: "pcmmeasuringpoint:ExternalCallActionMeasuringPoint",
                    label: format!("ExternalCall {name} [{id}]"),
                    refs: vec![("externalCall", None, format!("{repo}#{id}"))],
                    replica: None,
                    attrs: vec![],
                },
                format!("RT external call {name}"),
                vec![RESPONSE_TIME],
            ));
        }
    }
    for (ci, c) in m.containers.iter().enumerate() {
        for (ri, r) in c.res.iter().enumerate() {
            for i in 0..r.replicas.max(1) {
                for w in &m.windows {
                    if w.target == WindowTarget::Utilisation(ci, ri, i) {
                        extras.entry(mps.len()).or_default().push((
                            UTILIZATION,
                            "monitorrepository:TimeDriven",
                            w.len,
                            w.inc,
                        ));
                    }
                }
                let metrics = if i == 0 {
                    vec![STATE_ACTIVE, RESOURCE_DEMAND]
                } else {
                    vec![STATE_ACTIVE]
                };
                mps.push((
                    Mp {
                        ty: "pcmmeasuringpoint:ActiveResourceMeasuringPoint",
                        label: format!("{} [{i}] in {}", r.ty.name(), c.name),
                        refs: vec![("activeResource", None, format!("{env}#{}", r.id))],
                        replica: (i > 0).then_some(i),
                        attrs: vec![],
                    },
                    format!("resource {}.{} #{i}", c.name, r.ty.name()),
                    metrics,
                ));
            }
        }
        // nested containers after the container's own resources (refsim `Monitors.addContainer`)
        if let Some((p, nc, _)) = &m.nested
            && *p == ci
        {
            for r in &nc.res {
                mps.push((
                    Mp {
                        ty: "pcmmeasuringpoint:ActiveResourceMeasuringPoint",
                        label: format!("{} [0] in {}", r.ty.name(), nc.name),
                        refs: vec![("activeResource", None, format!("{env}#{}", r.id))],
                        replica: None,
                        attrs: vec![],
                    },
                    format!("resource {}.{} #0", nc.name, r.ty.name()),
                    vec![STATE_ACTIVE, RESOURCE_DEMAND],
                ));
            }
        }
    }
    for (ac_id, ac_name, comp) in &passive {
        if let Component::Basic(b) = &m.components[*comp] {
            for p in &b.passive {
                let file = if m.assemblies.iter().any(|a| &a.id == ac_id) {
                    &sys
                } else {
                    &repo
                };
                mps.push((
                    Mp {
                        ty: "pcmmeasuringpoint:AssemblyPassiveResourceMeasuringPoint",
                        label: format!("Passive Resource: {ac_name}.{}", p.name),
                        refs: vec![
                            ("assembly", None, format!("{file}#{ac_id}")),
                            ("passiveResource", None, format!("{repo}#{}", p.id)),
                        ],
                        replica: None,
                        attrs: vec![],
                    },
                    format!("passive {ac_name}.{}", p.name),
                    vec![WAITING_TIME, HOLDING_TIME, STATE_PASSIVE],
                ));
            }
        }
    }
    // Reconfigurator monitors (with `triggers`): reconfiguration time, number of containers
    if let Some(uri) = &m.reconf_monitor {
        let attrs = vec![
            ("measuringPoint", "Reconfigurations".to_string()),
            ("resourceURI", uri.clone()),
        ];
        mps.push((
            Mp {
                ty: "simulizarmeasuringpoint:ReconfigurationMeasuringPoint",
                label: "Reconfigurations".into(),
                refs: vec![],
                replica: None,
                attrs,
            },
            "Reconfigurations".into(),
            vec![RECONFIGURATION_TIME],
        ));
    }
    if m.container_count_monitor {
        mps.push((
            Mp {
                ty: "pcmmeasuringpoint:ResourceEnvironmentMeasuringPoint",
                label: format!("Resource Environment: {}", m.name),
                refs: vec![("resourceEnvironment", None, format!("{env}#/"))],
                replica: None,
                attrs: vec![],
            },
            "Number of resource containers".into(),
            vec![NUMBER_OF_RESOURCE_CONTAINERS],
        ));
    }
    // measuring point repository
    let mut x = X::new();
    x.s.push_str(&format!(
        "<org.palladiosimulator.edp2.models:MeasuringPointRepository xmi:version=\"2.0\" \
         xmlns:xmi=\"http://www.omg.org/XMI\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:org.palladiosimulator.edp2.models=\"http://palladiosimulator.org/EDP2/MeasuringPoint/1.0\" \
         xmlns:pcmmeasuringpoint=\"http://palladiosimulator.org/PCM/MeasuringPoint/1.0\" \
         xmlns:simulizarmeasuringpoint=\"http://palladiosimulator.org/simulizar/measuringpoint\" \
         xmlns:repository=\"http://palladiosimulator.org/PalladioComponentModel/Repository/5.2\" id=\"_{}_mpr\">\n",
        esc(n)
    ));
    for (mp, _, _) in &mps {
        let rep = mp.replica.map(|r| r.to_string());
        let mut attrs = vec![
            ("xsi:type", mp.ty),
            ("stringRepresentation", mp.label.as_str()),
        ];
        if let Some(r) = &rep {
            attrs.push(("replicaID", r));
        }
        for (k, v) in &mp.attrs {
            attrs.push((k, v));
        }
        x.tag(1, "measuringPoints", &attrs, false);
        for (tag, xsi, href) in &mp.refs {
            x.href(2, tag, *xsi, href);
        }
        x.end(1, "measuringPoints");
    }
    x.s.push_str("</org.palladiosimulator.edp2.models:MeasuringPointRepository>\n");
    let mpr = x.s;
    // monitor repository
    let mut x = X::new();
    x.s.push_str(&format!(
        "<monitorrepository:MonitorRepository xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:metricspec=\"http://palladiosimulator.org/MetricSpec/1.0\" \
         xmlns:monitorrepository=\"http://palladiosimulator.org/MonitorRepository/1.0\" \
         xmlns:pcmmeasuringpoint=\"http://palladiosimulator.org/PCM/MeasuringPoint/1.0\" \
         xmlns:simulizarmeasuringpoint=\"http://palladiosimulator.org/simulizar/measuringpoint\" \
         id=\"_{}_monrepo\" entityName=\"refsim default monitors\">\n",
        esc(n)
    ));
    for (k, (mp, label, metrics)) in mps.iter().enumerate() {
        let i = k + 1;
        x.tag(
            1,
            "monitors",
            &[("id", &format!("_{n}_mon{i}")), ("entityName", label)],
            false,
        );
        for met in metrics {
            // a TimeDrivenAggregation replaces the FeedThrough of the same metric (two calculators
            // for one measuring point and metric abort the reference)
            if extras
                .get(&k)
                .is_some_and(|e| e.iter().any(|(m2, ..)| m2 == met))
            {
                continue;
            }
            if *met == RESPONSE_TIME
                && let Some(a) = aggs.get(&k)
            {
                let sid = format!("_{n}_ms{i}_agg");
                x.tag(2, "measurementSpecifications", &spec_attrs(m, &sid), false);
                x.href(
                    3,
                    "metricDescription",
                    Some("metricspec:NumericalBaseMetricDescription"),
                    &format!("{METRICS}{met}"),
                );
                let id = format!("_{n}_agg{i}");
                let freq = a.frequency.to_string();
                let (ty, key, val) = if a.fixed {
                    (
                        "monitorrepository:FixedSizeAggregation",
                        "numberOfMeasurements",
                        a.number_of_measurements.to_string(),
                    )
                } else {
                    (
                        "monitorrepository:VariableSizeAggregation",
                        "retrospectionLength",
                        crate::javafmt::to_string(a.retrospection_length),
                    )
                };
                x.tag(
                    3,
                    "processingType",
                    &[
                        ("xsi:type", ty),
                        ("id", id.as_str()),
                        ("frequency", freq.as_str()),
                        (key, val.as_str()),
                    ],
                    false,
                );
                x.tag(
                    4,
                    "statisticalCharacterization",
                    &[("xsi:type", "monitorrepository:ArithmeticMean")],
                    true,
                );
                x.end(3, "processingType");
                x.end(2, "measurementSpecifications");
                continue;
            }
            let short = &met[1..5];
            let sid = format!("_{n}_ms{i}_{short}");
            let mut attrs = spec_attrs(m, &sid);
            if *met == RECONFIGURATION_TIME {
                // a triggering reconfiguration-time spec aborts the reference after the stop
                // (REF-11); covered by corpus-fuzz/l_ref_reconf_rescheduled_after_stop
                attrs.retain(|a| a.0 != "triggersSelfAdaptations");
                attrs.push(("triggersSelfAdaptations", "false"));
            }
            x.tag(2, "measurementSpecifications", &attrs, false);
            x.href(
                3,
                "metricDescription",
                Some("metricspec:NumericalBaseMetricDescription"),
                &format!("{METRICS}{met}"),
            );
            x.tag(
                3,
                "processingType",
                &[
                    ("xsi:type", "monitorrepository:FeedThrough"),
                    ("id", &format!("_{n}_ft{i}_{short}")),
                ],
                true,
            );
            x.end(2, "measurementSpecifications");
        }
        for (j, (met, ptype, len, inc)) in extras.get(&k).into_iter().flatten().enumerate() {
            let sid = format!("_{n}_ms{i}_w{j}");
            x.tag(2, "measurementSpecifications", &spec_attrs(m, &sid), false);
            x.href(
                3,
                "metricDescription",
                Some("metricspec:NumericalBaseMetricDescription"),
                &format!("{METRICS}{met}"),
            );
            let (l, c) = (
                crate::javafmt::to_string(*len),
                crate::javafmt::to_string(*inc),
            );
            let id = format!("_{n}_pt{i}_w{j}");
            let attrs = [
                ("xsi:type", *ptype),
                ("id", id.as_str()),
                ("windowLength", l.as_str()),
                ("windowIncrement", c.as_str()),
            ];
            if *ptype == "monitorrepository:TimeDrivenAggregation" {
                x.tag(3, "processingType", &attrs, false);
                x.tag(
                    4,
                    "statisticalCharacterization",
                    &[("xsi:type", "monitorrepository:ArithmeticMean")],
                    true,
                );
                x.end(3, "processingType");
            } else {
                x.tag(3, "processingType", &attrs, true);
            }
            x.end(2, "measurementSpecifications");
        }
        x.href(
            2,
            "measuringPoint",
            Some(mp.ty),
            &format!("{n}.measuringpoint#//@measuringPoints.{k}"),
        );
        x.end(1, "monitors");
    }
    x.s.push_str("</monitorrepository:MonitorRepository>\n");
    (mpr, x.s)
}

/// Attributes of a measurement specification: id and `triggersSelfAdaptations` (omitted for the
/// EMF default `true`).
fn spec_attrs<'a>(m: &GenModel, id: &'a str) -> Vec<(&'static str, &'a str)> {
    let mut v = vec![("id", id)];
    match m.triggers.value(id) {
        Some(true) => v.push(("triggersSelfAdaptations", "true")),
        Some(false) => v.push(("triggersSelfAdaptations", "false")),
        None => {}
    }
    v
}

/// All files of the model as `(file name, content)`.
pub fn files(m: &GenModel) -> Vec<(String, String)> {
    let n = &m.name;
    let (mpr, mon) = monitors(m);
    vec![
        (format!("{n}.repository"), repository(m)),
        (format!("{n}.system"), system(m)),
        (format!("{n}.resourceenvironment"), resenv(m)),
        (format!("{n}.allocation"), allocation(m)),
        (format!("{n}.usagemodel"), usagemodel(m)),
        (format!("{n}.measuringpoint"), mpr),
        (format!("{n}.monitorrepository"), mon),
        ("run.json".to_string(), m.run.to_json()),
        (
            "FEATURES.txt".to_string(),
            format!("{}\n", m.features.join(", ")),
        ),
    ]
}

/// Writes the model into `dir` (created; existing model files are replaced).
pub fn write_model(m: &GenModel, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, content) in files(m) {
        std::fs::write(dir.join(name), content)?;
    }
    Ok(())
}
