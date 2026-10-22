//! Path monitoring (`SimConfig::sync_paths`, `SimConfig::call_counts`;
//! `docs/spec/measurements.md` MEAS-13), on a generated model where the answer is known:
//!
//! * scenario `main` (open) calls A on container CA; A branches with probability 0.3 to B (CB),
//!   which calls E (CE); then A forks: asynchronously C (CC), which calls F (CF), and
//!   synchronously D (CD). So every session's path holds CA and CD, 30 % hold CB and CE (always
//!   together), none CC or CF;
//! * scenario `direct` (closed, 2 users) calls D only: every iteration's path is {CD};
//! * call counts: A's call of B in 30 % of A's executions, of C and D in all, C calls F as
//!   often as C runs;
//! * the options change nothing else (same trace), and paused, forked and snapshotted runs
//!   continue with the same records.

use simoxide_sim::snapshot::{Encoding, SnapshotOptions};
use simoxide_sim::{
    CompiledModel, FlavourConfig, Mode, Outputs, Pause, RunSpec, SessionPaths, SimConfig,
    Simulation,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

const S: i64 = 1_000_000_000;
const CPU: &str = "pathmap://PCM_MODELS/Palladio.resourcetype#_oro4gG3fEdy4YaaT-RYrLQ";
const NS: &str = r#"xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#;

/// One action of a generated SEFF behaviour.
enum Act {
    Work(&'static str),
    Call(&'static str),
    /// A probabilistic branch: (probability, actions) per transition.
    Branch(Vec<(f64, Vec<Act>)>),
    /// A fork: asynchronous behaviours, synchronous behaviours (with a synchronisation point).
    Fork(Vec<Vec<Act>>, Vec<Vec<Act>>),
}

struct Gen {
    n: usize,
    /// (component, callee) per required role.
    out: String,
}

impl Gen {
    fn id(&mut self, p: &str) -> String {
        self.n += 1;
        format!("_{p}{}", self.n)
    }

    /// Steps of a behaviour (start, actions, stop) in component `c`.
    fn steps(&mut self, c: &str, acts: &[Act]) -> String {
        let mut ids: Vec<String> = (0..acts.len() + 2).map(|_| self.id("a")).collect();
        for (k, a) in acts.iter().enumerate() {
            if let Act::Call(t) = a {
                ids[k + 1] = format!("call_{c}_{t}");
            }
        }
        let mut s = String::new();
        let link = |i: usize| {
            let mut x = String::new();
            if i > 0 {
                x += &format!(r#" predecessor_AbstractAction="{}""#, ids[i - 1]);
            }
            if i + 1 < ids.len() {
                x += &format!(r#" successor_AbstractAction="{}""#, ids[i + 1]);
            }
            x
        };
        s += &format!(
            r#"<steps_Behaviour xsi:type="seff:StartAction" id="{}"{}/>"#,
            ids[0],
            link(0)
        );
        for (k, a) in acts.iter().enumerate() {
            let i = k + 1;
            let id = &ids[i];
            match a {
                Act::Work(d) => {
                    s += &format!(
                        r#"<steps_Behaviour xsi:type="seff:InternalAction" id="{id}" entityName="work"{}><resourceDemand_Action><specification_ParametericResourceDemand specification="{d}"/><requiredResource_ParametricResourceDemand href="{CPU}"/></resourceDemand_Action></steps_Behaviour>"#,
                        link(i)
                    );
                }
                Act::Call(t) => {
                    s += &format!(
                        r#"<steps_Behaviour xsi:type="seff:ExternalCallAction" id="{id}" entityName="call {t}"{} calledService_ExternalService="sig_{t}" role_ExternalService="req_{c}_{t}"/>"#,
                        link(i)
                    );
                }
                Act::Branch(ts) => {
                    s += &format!(
                        r#"<steps_Behaviour xsi:type="seff:BranchAction" id="{id}" entityName="branch"{}>"#,
                        link(i)
                    );
                    for (p, body) in ts {
                        let (t, b) = (self.id("t"), self.id("b"));
                        let inner = self.steps(c, body);
                        s += &format!(
                            r#"<branches_Branch xsi:type="seff:ProbabilisticBranchTransition" id="{t}" branchProbability="{p}"><branchBehaviour_BranchTransition id="{b}">{inner}</branchBehaviour_BranchTransition></branches_Branch>"#
                        );
                    }
                    s += "</steps_Behaviour>";
                }
                Act::Fork(asy, syn) => {
                    s += &format!(
                        r#"<steps_Behaviour xsi:type="seff:ForkAction" id="{id}" entityName="fork"{}>"#,
                        link(i)
                    );
                    for body in asy {
                        let b = self.id("fb");
                        let inner = self.steps(c, body);
                        s += &format!(
                            r#"<asynchronousForkedBehaviours_ForkAction id="{b}">{inner}</asynchronousForkedBehaviours_ForkAction>"#
                        );
                    }
                    if !syn.is_empty() {
                        let sp = self.id("sp");
                        s += &format!(r#"<synchronisingBehaviours_ForkAction id="{sp}">"#);
                        for body in syn {
                            let b = self.id("fb");
                            let inner = self.steps(c, body);
                            s += &format!(
                                r#"<synchronousForkedBehaviours_SynchronisationPoint id="{b}">{inner}</synchronousForkedBehaviours_SynchronisationPoint>"#
                            );
                        }
                        s += "</synchronisingBehaviours_ForkAction>";
                    }
                    s += "</steps_Behaviour>";
                }
            }
        }
        let last = ids.len() - 1;
        s += &format!(
            r#"<steps_Behaviour xsi:type="seff:StopAction" id="{}"{}/>"#,
            ids[last],
            link(last)
        );
        s
    }
}

fn calls(acts: &[Act], out: &mut Vec<&'static str>) {
    for a in acts {
        match a {
            Act::Call(t) => out.push(t),
            Act::Branch(ts) => ts.iter().for_each(|(_, b)| calls(b, out)),
            Act::Fork(a, s) => a.iter().chain(s).for_each(|b| calls(b, out)),
            Act::Work(_) => {}
        }
    }
}

/// The PCM files: components (name, SEFF) each on its own container `C<name>`; the system
/// provides A and D; scenario `main` (open, Exp(`rate`)) calls A, `direct` (closed, 2 users,
/// think time Exp(2)) calls D.
fn files(rate: f64, comps: &[(&'static str, Vec<Act>)]) -> Vec<(String, String)> {
    let mut g = Gen {
        n: 0,
        out: String::new(),
    };
    let mut repo = String::new();
    for (c, acts) in comps {
        let mut req = Vec::new();
        calls(acts, &mut req);
        let body = g.steps(c, acts);
        repo += &format!(
            r#"<components__Repository xsi:type="repository:BasicComponent" id="comp_{c}" entityName="{c}"><providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole" id="prov_{c}" providedInterface__OperationProvidedRole="if_{c}"/>"#
        );
        for t in &req {
            repo += &format!(
                r#"<requiredRoles_InterfaceRequiringEntity xsi:type="repository:OperationRequiredRole" id="req_{c}_{t}" requiredInterface__OperationRequiredRole="if_{t}"/>"#
            );
        }
        repo += &format!(
            r#"<serviceEffectSpecifications__BasicComponent xsi:type="seff:ResourceDemandingSEFF" id="seff_{c}" describedService__SEFF="sig_{c}">{body}</serviceEffectSpecifications__BasicComponent></components__Repository>"#
        );
    }
    for (c, _) in comps {
        repo += &format!(
            r#"<interfaces__Repository xsi:type="repository:OperationInterface" id="if_{c}" entityName="I{c}"><signatures__OperationInterface id="sig_{c}" entityName="op{c}"/></interfaces__Repository>"#
        );
    }
    let _ = &g.out;
    let repo = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><repository:Repository {NS} xmlns:repository="http://palladiosimulator.org/PalladioComponentModel/Repository/5.2" xmlns:seff="http://palladiosimulator.org/PalladioComponentModel/SEFF/5.2" id="repo" entityName="paths">{repo}</repository:Repository>"#
    );
    let mut sys = String::new();
    let mut alloc = String::new();
    let mut env = String::new();
    for (c, acts) in comps {
        sys += &format!(
            r#"<assemblyContexts__ComposedStructure id="ac_{c}" entityName="Assembly_{c}"><encapsulatedComponent__AssemblyContext xsi:type="repository:BasicComponent" href="paths.repository#comp_{c}"/></assemblyContexts__ComposedStructure>"#
        );
        let mut req = Vec::new();
        calls(acts, &mut req);
        for t in req {
            sys += &format!(
                r#"<connectors__ComposedStructure xsi:type="composition:AssemblyConnector" id="conn_{c}_{t}" requiringAssemblyContext_AssemblyConnector="ac_{c}" providingAssemblyContext_AssemblyConnector="ac_{t}"><providedRole_AssemblyConnector href="paths.repository#prov_{t}"/><requiredRole_AssemblyConnector href="paths.repository#req_{c}_{t}"/></connectors__ComposedStructure>"#
            );
        }
        alloc += &format!(
            r#"<allocationContexts_Allocation id="alc_{c}"><resourceContainer_AllocationContext href="paths.resourceenvironment#C{c}"/><assemblyContext_AllocationContext href="paths.system#ac_{c}"/></allocationContexts_Allocation>"#
        );
        env += &format!(
            r#"<resourceContainer_ResourceEnvironment id="C{c}" entityName="C{c}"><activeResourceSpecifications_ResourceContainer id="cpu_{c}" numberOfReplicas="4"><schedulingPolicy href="pathmap://PCM_MODELS/Palladio.resourcetype#ProcessorSharing"/><activeResourceType_ActiveResourceSpecification href="{CPU}"/><processingRate_ProcessingResourceSpecification specification="1.0"/></activeResourceSpecifications_ResourceContainer></resourceContainer_ResourceEnvironment>"#
        );
    }
    for e in ["A", "D"] {
        sys += &format!(
            r#"<connectors__ComposedStructure xsi:type="composition:ProvidedDelegationConnector" id="deleg_{e}" outerProvidedRole_ProvidedDelegationConnector="sysprov_{e}" assemblyContext_ProvidedDelegationConnector="ac_{e}"><innerProvidedRole_ProvidedDelegationConnector href="paths.repository#prov_{e}"/></connectors__ComposedStructure><providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole" id="sysprov_{e}"><providedInterface__OperationProvidedRole href="paths.repository#if_{e}"/></providedRoles_InterfaceProvidingEntity>"#
        );
    }
    let sys = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><system:System {NS} xmlns:composition="http://palladiosimulator.org/PalladioComponentModel/Core/Composition/5.2" xmlns:repository="http://palladiosimulator.org/PalladioComponentModel/Repository/5.2" xmlns:system="http://palladiosimulator.org/PalladioComponentModel/System/5.2" id="system" entityName="paths">{sys}</system:System>"#
    );
    let alloc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><allocation:Allocation {NS} xmlns:allocation="http://palladiosimulator.org/PalladioComponentModel/Allocation/5.2" id="allocation"><targetResourceEnvironment_Allocation href="paths.resourceenvironment#/"/><system_Allocation href="paths.system#system"/>{alloc}</allocation:Allocation>"#
    );
    let all: Vec<String> = comps.iter().map(|(c, _)| format!("C{c}")).collect();
    let env = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><resourceenvironment:ResourceEnvironment {NS} xmlns:resourceenvironment="http://palladiosimulator.org/PalladioComponentModel/ResourceEnvironment/5.2" entityName="paths"><linkingResources__ResourceEnvironment id="lan" entityName="LAN" connectedResourceContainers_LinkingResource="{}"><communicationLinkResourceSpecifications_LinkingResource id="lan_spec"><communicationLinkResourceType_CommunicationLinkResourceSpecification href="pathmap://PCM_MODELS/Palladio.resourcetype#_o3sScH2AEdyH8uerKnHYug"/><latency_CommunicationLinkResourceSpecification specification="0.0"/><throughput_CommunicationLinkResourceSpecification specification="1.0E9"/></communicationLinkResourceSpecifications_LinkingResource></linkingResources__ResourceEnvironment>{env}</resourceenvironment:ResourceEnvironment>"#,
        all.join(" ")
    );
    let scen = |name: &str, e: &str, workload: String| {
        format!(
            r#"<usageScenario_UsageModel id="us_{name}" entityName="{name}"><scenarioBehaviour_UsageScenario id="sb_{name}"><actions_ScenarioBehaviour xsi:type="usagemodel:Start" id="ua_{name}_0" successor="ua_{name}_1"/><actions_ScenarioBehaviour xsi:type="usagemodel:EntryLevelSystemCall" id="elsc_{name}" successor="ua_{name}_2" predecessor="ua_{name}_0"><providedRole_EntryLevelSystemCall href="paths.system#sysprov_{e}"/><operationSignature__EntryLevelSystemCall href="paths.repository#sig_{e}"/></actions_ScenarioBehaviour><actions_ScenarioBehaviour xsi:type="usagemodel:Stop" id="ua_{name}_2" predecessor="elsc_{name}"/></scenarioBehaviour_UsageScenario>{workload}</usageScenario_UsageModel>"#
        )
    };
    let usage = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><usagemodel:UsageModel {NS} xmlns:usagemodel="http://palladiosimulator.org/PalladioComponentModel/UsageModel/5.2">{}{}</usagemodel:UsageModel>"#,
        scen(
            "main",
            "A",
            format!(r#"<workload_UsageScenario xsi:type="usagemodel:OpenWorkload"><interArrivalTime_OpenWorkload specification="Exp({rate})"/></workload_UsageScenario>"#)
        ),
        scen(
            "direct",
            "D",
            r#"<workload_UsageScenario xsi:type="usagemodel:ClosedWorkload" population="2"><thinkTime_ClosedWorkload specification="Exp(2.0)"/></workload_UsageScenario>"#.to_string()
        ),
    );
    vec![
        ("paths.repository".into(), repo),
        ("paths.system".into(), sys),
        ("paths.allocation".into(), alloc),
        ("paths.resourceenvironment".into(), env),
        ("paths.usagemodel".into(), usage),
    ]
}

fn model() -> CompiledModel {
    use Act::*;
    let comps = vec![
        (
            "A",
            vec![
                Work("Exp(100.0)"),
                Branch(vec![(0.3, vec![Call("B")]), (0.7, vec![Work("0.001")])]),
                Fork(vec![vec![Call("C")]], vec![vec![Call("D")]]),
            ],
        ),
        ("B", vec![Work("Exp(100.0)"), Call("E")]),
        ("C", vec![Work("Exp(100.0)"), Call("F")]),
        ("D", vec![Work("Exp(50.0)")]),
        ("E", vec![Work("Exp(100.0)")]),
        ("F", vec![Work("Exp(100.0)")]),
    ];
    let f = files(20.0, &comps);
    let m = RunSpec {
        usagemodel: Some("paths.usagemodel".into()),
        ..RunSpec::default()
    }
    .load_model_memory(&f)
    .unwrap();
    CompiledModel::compile_flavour(m, FlavourConfig::SIMULIZAR).unwrap()
}

fn cfg(on: bool, until_s: i64) -> SimConfig {
    SimConfig {
        seed: 3,
        max_sim_time: until_s,
        mode: Mode::Fast,
        usage_streams: true,
        sync_paths: on,
        call_counts: on,
        ..Default::default()
    }
}

/// Sessions per scenario id and container-id set.
fn sessions(cm: &CompiledModel, v: &[SessionPaths]) -> BTreeMap<(String, Vec<String>), u64> {
    v.iter()
        .map(|s| {
            let ids = s
                .containers
                .iter()
                .map(|&c| cm.model.containers[c].id.to_string())
                .collect();
            ((s.scenario_id.clone(), ids), s.sessions)
        })
        .collect()
}

fn count_of(cm: &CompiledModel, counts: &[u64], id: &str) -> u64 {
    let i = cm
        .action_ids
        .iter()
        .position(|a| &**a == id)
        .unwrap_or_else(|| panic!("no action {id}"));
    counts.get(i).copied().unwrap_or(0)
}

#[test]
fn sessions_record_their_synchronous_paths() {
    let cm = model();
    let mut sim =
        Simulation::<simoxide_sim::Fast>::create(&cm, cfg(true, 400), Outputs::default()).unwrap();
    sim.run_until(Pause::Never).unwrap();
    let st = sim.state();
    let got = sessions(&cm, &st.sync_paths(&cm).unwrap());
    let counts = st.call_counts().unwrap().to_vec();
    let mut main = 0u64;
    let mut with_b = 0u64;
    let mut direct = 0u64;
    for ((scen, set), n) in &got {
        let has = |c: &str| set.iter().any(|x| x == c);
        match scen.as_str() {
            "us_main" => {
                main += n;
                assert!(has("CA") && has("CD"), "{set:?}");
                assert!(
                    !has("CC") && !has("CF"),
                    "asynchronous work is off the path: {set:?}"
                );
                assert_eq!(has("CB"), has("CE"), "{set:?}");
                if has("CB") {
                    with_b += n;
                }
                assert_eq!(set.len(), if has("CB") { 4 } else { 2 });
            }
            "us_direct" => {
                direct += n;
                assert_eq!(set, &vec!["CD".to_string()]);
            }
            other => panic!("scenario {other}"),
        }
    }
    // 20 sessions/s for 400 s
    assert!(main > 7000, "{main} sessions");
    assert!(direct > 100, "{direct} sessions");
    let share = with_b as f64 / main as f64;
    let sd = (0.3 * 0.7 / main as f64).sqrt();
    assert!((share - 0.3).abs() < 4.0 * sd, "share of B {share}");
    // call counts: started calls (the last few may not have completed)
    let (b, c, d, f) = (
        count_of(&cm, &counts, "call_A_B"),
        count_of(&cm, &counts, "call_A_C"),
        count_of(&cm, &counts, "call_A_D"),
        count_of(&cm, &counts, "call_C_F"),
    );
    assert_eq!(c, d);
    assert!(c >= main && c <= main + 50, "{c} vs {main}");
    assert!(f <= c && f + 50 >= c, "{f} vs {c}");
    assert_eq!(count_of(&cm, &counts, "call_B_E"), b);
    let r = b as f64 / c as f64;
    assert!((r - 0.3).abs() < 0.03, "{r}");
}

#[derive(Clone, Default)]
struct Buf(Rc<RefCell<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn off_by_default_and_the_run_is_unchanged() {
    let cm = model();
    let d = SimConfig::default();
    assert!(!d.sync_paths && !d.call_counts);
    let trace = |on: bool| {
        let t = Buf::default();
        let out = Outputs {
            trace: Some(Box::new(t.clone())),
            tape: None,
        };
        let mut sim = Simulation::<simoxide_sim::Fast>::create(&cm, cfg(on, 60), out).unwrap();
        sim.run_until(Pause::Never).unwrap();
        assert_eq!(sim.state().sync_paths(&cm).is_some(), on);
        assert_eq!(sim.state().call_counts().is_some(), on);
        let r = sim.finish().unwrap();
        (
            String::from_utf8(t.0.borrow().clone()).unwrap(),
            r.measurements.to_csv(),
            r.events,
        )
    };
    let (a, b) = (trace(false), trace(true));
    assert!(a.0.len() > 10_000);
    assert_eq!(a, b);
}

#[test]
fn paused_forked_and_snapshotted_runs_record_the_same() {
    let cm = model();
    let fp = |sim: Simulation<simoxide_sim::Fast>| {
        let mut sim = sim;
        sim.run_until(Pause::Never).unwrap();
        let st = sim.state();
        format!(
            "{:?}\n{:?}",
            sessions(&cm, &st.sync_paths(&cm).unwrap()),
            st.call_counts().unwrap()
        )
    };
    let want = fp(Simulation::create(&cm, cfg(true, 90), Outputs::default()).unwrap());
    for how in ["direct", "fork", "postcard", "json"] {
        let mut sim =
            Simulation::<simoxide_sim::Fast>::create(&cm, cfg(true, 90), Outputs::default())
                .unwrap();
        sim.run_until(Pause::Before(41 * S)).unwrap();
        let cont = match how {
            "direct" => sim,
            "fork" => sim.fork(Outputs::default()).unwrap(),
            enc => {
                let encoding = if enc == "json" {
                    Encoding::Json
                } else {
                    Encoding::Postcard
                };
                let mut bytes = Vec::new();
                sim.snapshot_with(&mut bytes, &SnapshotOptions { encoding }, Vec::new())
                    .unwrap();
                Simulation::<simoxide_sim::Fast>::restore_from(&bytes[..], &cm, Outputs::default())
                    .unwrap()
            }
        };
        assert_eq!(fp(cont), want, "{how}");
    }
}

#[test]
fn the_slingshot_exact_engine_refuses() {
    let m = RunSpec {
        usagemodel: Some("paths.usagemodel".into()),
        ..RunSpec::default()
    }
    .load_model_memory(&files(
        20.0,
        &[
            ("A", vec![Act::Work("0.01")]),
            ("D", vec![Act::Work("0.01")]),
        ],
    ))
    .unwrap();
    let ss = CompiledModel::compile_flavour(m, FlavourConfig::slingshot_exact()).unwrap();
    let mut c = cfg(true, 10);
    c.usage_streams = false;
    let e = simoxide_sim::run(&ss, c, Outputs::default()).unwrap_err();
    assert!(e.message.contains("path monitoring"), "{}", e.message);
}
