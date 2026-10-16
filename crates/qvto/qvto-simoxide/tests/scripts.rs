//! The PCM roles of the store contract: random scripts of model-API operations on PCM classes
//! (create, set, unset, add, insert, remove, set at, move, clear, `setEList`, roots, delete; with
//! wrong types, nulls and bad indexes) run on the adapter (objects in a `raw::Graph`) and on
//! `qvto_emf::Graph` with the PCM metamodels loaded from their `.ecore` files. `qvto_emf` is
//! checked against EMF itself (`crates/qvto-emf/tests/scripts.rs`), so equal results and equal
//! final states mean EMF's semantics on SimOxide's graph. Needs `target/pcm` (skipped otherwise).

mod registry;

use qvto_emf::ResourceSet;
use qvto_model::{
    ClassId, Classifier, DataKind, Datum, EnumId, FeatureId, FeatureKind, MetaRegistry,
    ModelResult, ModelStore, ResourceId,
};
use qvto_simoxide::{Journal, Session, SimStore};
use simoxide_model::raw;
use std::collections::HashMap;

const SCRIPTS: u64 = 300;
const OPS: usize = 80;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

/// The PCM classes the scripts use most (resource environment, allocation, system, the
/// repository's components), with any other concrete class now and then.
const FOCUS: &[&str] = &[
    "ResourceEnvironment",
    "ResourceContainer",
    "ProcessingResourceSpecification",
    "LinkingResource",
    "CommunicationLinkResourceSpecification",
    "Allocation",
    "AllocationContext",
    "System",
    "AssemblyContext",
    "AssemblyConnector",
    "ProvidedDelegationConnector",
    "Repository",
    "BasicComponent",
    "OperationInterface",
    "OperationSignature",
    "OperationProvidedRole",
    "OperationRequiredRole",
    "PCMRandomVariable",
    "ResourceDemandingSEFF",
    "InternalAction",
    "ParametricResourceDemand",
    "UsageModel",
    "UsageScenario",
    "ClosedWorkload",
    "MonitorRepository",
    "Monitor",
    "MeasurementSpecification",
];

/// Ids of the registry of `qvto_emf` for the adapter's (SimOxide's) ids, by names.
struct IdMap {
    class: HashMap<ClassId, ClassId>,
    feature: HashMap<FeatureId, FeatureId>,
    enums: HashMap<EnumId, EnumId>,
}

fn id_map(ours: &MetaRegistry, theirs: &MetaRegistry) -> IdMap {
    let mut m = IdMap {
        class: HashMap::new(),
        feature: HashMap::new(),
        enums: HashMap::new(),
    };
    for c in ours.classes() {
        let ci = ours.class(c);
        let Some(ns) = ours.package(ci.package).ns_uri.as_deref() else {
            continue;
        };
        let Some(p) = theirs.package_by_ns_uri(ns) else {
            continue;
        };
        let Some(c2) = theirs.class_by_name(p, &ci.name) else {
            continue;
        };
        m.class.insert(c, c2);
        for &f in &ci.features {
            let f2 = theirs.feature_by_name(c2, &ours.feature(f).name).unwrap();
            m.feature.insert(f, f2);
        }
    }
    for p in ours.packages() {
        let pi = ours.package(p);
        let Some(p2) = pi.ns_uri.as_deref().and_then(|n| theirs.package_by_ns_uri(n)) else {
            continue;
        };
        for &k in &pi.classifiers {
            if let Classifier::Enum(e) = k
                && let Some(Classifier::Enum(e2)) =
                    theirs.classifier_by_name(p2, &ours.enumeration(e).name)
            {
                m.enums.insert(e, e2);
            }
        }
    }
    m
}

/// An operation in the adapter's ids; objects by creation index.
#[derive(Clone, Debug)]
enum Op {
    Create(ClassId),
    Set(usize, FeatureId, Val),
    Unset(usize, FeatureId),
    Add(usize, FeatureId, Val, Option<usize>),
    Remove(usize, FeatureId, Val),
    RemoveAt(usize, FeatureId, usize),
    SetAt(usize, FeatureId, usize, Val),
    Clear(usize, FeatureId),
    Move(usize, FeatureId, usize, usize),
    SetList(usize, FeatureId, Vec<Val>),
    AddRoot(usize, usize, Option<usize>),
    RemoveRoot(usize, usize),
    Delete(usize),
}

#[derive(Clone, Debug)]
enum Val {
    Null,
    Obj(usize),
    Str(String),
    Int(i32),
    Long(i64),
    Double(f64),
    Bool(bool),
    Enum(EnumId, u32),
}

/// One side of the comparison: a store and the translation of ids.
struct Side<'m, S: ModelStore> {
    s: S,
    map: Option<&'m IdMap>,
    objs: Vec<S::Obj>,
    res: Vec<ResourceId>,
}

impl<S: ModelStore> Side<'_, S> {
    fn c(&self, c: ClassId) -> ClassId {
        self.map.map_or(c, |m| m.class[&c])
    }
    fn f(&self, f: FeatureId) -> FeatureId {
        self.map.map_or(f, |m| m.feature[&f])
    }
    fn d(&self, v: &Val) -> Datum<'static, S::Obj> {
        match v {
            Val::Null => Datum::Null,
            Val::Obj(i) => Datum::Obj(self.objs[*i]),
            Val::Str(s) => Datum::Str(Box::leak(s.clone().into_boxed_str())),
            Val::Int(i) => Datum::Int(*i),
            Val::Long(i) => Datum::Long(*i),
            Val::Double(x) => Datum::Double(*x),
            Val::Bool(b) => Datum::Bool(*b),
            Val::Enum(e, i) => Datum::Enum(self.map.map_or(*e, |m| m.enums[e]), *i),
        }
    }

    fn apply(&mut self, op: &Op) -> String {
        fn r<T: std::fmt::Debug>(x: ModelResult<T>) -> String {
            match x {
                Ok(v) => format!("Ok({v:?})"),
                Err(e) => format!("Err({})", format!("{e:?}").split(['(', ' ']).next().unwrap()),
            }
        }
        match op {
            Op::Create(c) => match self.s.create(self.c(*c)) {
                Ok(o) => {
                    self.objs.push(o);
                    "Ok".into()
                }
                Err(e) => r::<()>(Err(e)),
            },
            Op::Set(o, f, v) => {
                let d = self.d(v);
                r(self.s.set(self.objs[*o], self.f(*f), d))
            }
            Op::Unset(o, f) => r(self.s.unset(self.objs[*o], self.f(*f))),
            Op::Add(o, f, v, at) => {
                let d = self.d(v);
                r(self.s.add(self.objs[*o], self.f(*f), d, *at))
            }
            Op::Remove(o, f, v) => {
                let d = self.d(v);
                r(self.s.remove(self.objs[*o], self.f(*f), d))
            }
            Op::RemoveAt(o, f, i) => r(self.s.remove_at(self.objs[*o], self.f(*f), *i)),
            Op::SetAt(o, f, i, v) => {
                let d = self.d(v);
                r(self.s.set_at(self.objs[*o], self.f(*f), *i, d))
            }
            Op::Clear(o, f) => r(self.s.clear(self.objs[*o], self.f(*f))),
            Op::Move(o, f, to, from) => r(self.s.move_to(self.objs[*o], self.f(*f), *to, *from)),
            Op::SetList(o, f, vs) => {
                let ds: Vec<_> = vs.iter().map(|v| self.d(v)).collect();
                r(self.s.set_list(self.objs[*o], self.f(*f), &ds))
            }
            Op::AddRoot(r_, o, at) => r(self.s.add_root(self.res[*r_], self.objs[*o], *at)),
            Op::RemoveRoot(r_, o) => format!("{}", self.s.remove_root(self.res[*r_], self.objs[*o])),
            Op::Delete(o) => {
                self.s.delete(self.objs[*o]);
                "done".into()
            }
        }
    }

    /// Every feature of every object (objects by creation index), containers and resources.
    fn state(&self, ours: &MetaRegistry) -> String {
        let idx: HashMap<S::Obj, usize> = self.objs.iter().enumerate().map(|(i, o)| (*o, i)).collect();
        let show = |d: Datum<'_, S::Obj>| match d {
            Datum::Obj(x) => format!("#{}", idx.get(&x).map_or("?".into(), |i| i.to_string())),
            Datum::Enum(_, i) => format!("enum {i}"),
            d => format!("{d:?}"),
        };
        let mut out = String::new();
        for (i, &o) in self.objs.iter().enumerate() {
            let c = ours.class(self.class_back(o, ours));
            out.push_str(&format!(
                "#{i} {} container={:?} direct={:?} resource={:?}\n",
                c.name,
                self.s.container(o).map(|(p, f)| (idx.get(&p).copied(), self.s.meta().feature(f).name.to_string())),
                self.s.direct_resource(o).and_then(|r| self.res.iter().position(|x| *x == r)),
                self.s.resource_of(o).and_then(|r| self.res.iter().position(|x| *x == r)),
            ));
            for &f in c.all_features.iter() {
                let g = self.f(f);
                let name = &ours.feature(f).name;
                if ours.feature(f).is_many() {
                    let vals: Vec<String> = (0..self.s.len(o, g)).map(|k| show(self.s.get_at(o, g, k))).collect();
                    out.push_str(&format!("  {name} set={} {:?}\n", self.s.is_set(o, g), vals));
                } else {
                    out.push_str(&format!("  {name} set={} {}\n", self.s.is_set(o, g), show(self.s.get(o, g))));
                }
            }
        }
        for (k, r) in self.res.iter().enumerate() {
            let roots: Vec<String> = self.s.roots(*r).iter().map(|x| format!("#{:?}", idx.get(x))).collect();
            out.push_str(&format!("resource {k} {roots:?}\n"));
        }
        out
    }

    fn class_back(&self, o: S::Obj, ours: &MetaRegistry) -> ClassId {
        let c = self.s.class_of(o);
        match self.map {
            None => c,
            Some(m) => *m.class.iter().find(|(_, v)| **v == c).map(|(k, _)| k).unwrap_or_else(|| {
                panic!("class {}", ours.class_count())
            }),
        }
    }
}

fn random_value(rng: &mut Rng, meta: &MetaRegistry, f: FeatureId, objs: &[ClassId]) -> Val {
    let fi = meta.feature(f);
    if rng.chance(5) {
        return Val::Null;
    }
    if fi.kind == FeatureKind::Reference {
        let Classifier::Class(t) = fi.ty else { return Val::Null };
        let fitting: Vec<usize> = (0..objs.len()).filter(|&i| meta.is_subclass(objs[i], t)).collect();
        if !fitting.is_empty() && !rng.chance(10) {
            return Val::Obj(fitting[rng.below(fitting.len())]);
        }
        if objs.is_empty() {
            return Val::Null;
        }
        return Val::Obj(rng.below(objs.len()));
    }
    if rng.chance(5) {
        // a wrong kind now and then
        return match rng.below(3) {
            0 => Val::Str("x".into()),
            1 => Val::Int(1),
            _ => Val::Double(1.5),
        };
    }
    match fi.ty {
        Classifier::Enum(e) => {
            let n = meta.enumeration(e).literals.len() as u32;
            Val::Enum(e, rng.below(n as usize + 1) as u32 % n.max(1))
        }
        Classifier::DataType(d) => match meta.data_type(d).kind {
            DataKind::String => Val::Str(["a", "b", "c", "", "Rate"][rng.below(5)].into()),
            DataKind::Int => Val::Int(rng.below(4) as i32),
            DataKind::Long => Val::Long(rng.below(4) as i64),
            DataKind::Double => Val::Double([0.0, 1.0, 2.5, -0.0][rng.below(4)]),
            DataKind::Boolean => Val::Bool(rng.chance(50)),
            _ => Val::Str("1".into()),
        },
        Classifier::Class(_) => Val::Null,
    }
}

fn script(seed: u64, meta: &MetaRegistry, classes: &[ClassId], focus: &[ClassId]) -> Vec<Op> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut ops = Vec::new();
    let mut objs: Vec<ClassId> = Vec::new();
    for _ in 0..OPS {
        if objs.len() < 3 || rng.chance(12) {
            let c = if rng.chance(85) {
                focus[rng.below(focus.len())]
            } else {
                classes[rng.below(classes.len())]
            };
            ops.push(Op::Create(c));
            objs.push(c);
            continue;
        }
        let o = rng.below(objs.len());
        let feats: Vec<FeatureId> = meta.class(objs[o]).all_features.to_vec();
        if rng.chance(6) {
            ops.push(match rng.below(3) {
                0 => Op::AddRoot(rng.below(2), o, if rng.chance(20) { Some(rng.below(3)) } else { None }),
                1 => Op::RemoveRoot(rng.below(2), o),
                _ => Op::Delete(o),
            });
            continue;
        }
        if feats.is_empty() {
            continue;
        }
        // prefer references and changeable features
        let f = feats[rng.below(feats.len())];
        let fi = meta.feature(f);
        let v = random_value(&mut rng, meta, f, &objs);
        if !fi.is_many() {
            ops.push(if rng.chance(80) { Op::Set(o, f, v) } else { Op::Unset(o, f) });
            continue;
        }
        let n = 3; // index range for bad indexes
        ops.push(match rng.below(10) {
            0..=3 => Op::Add(o, f, v, if rng.chance(25) { Some(rng.below(n)) } else { None }),
            4 => Op::Remove(o, f, v),
            5 => Op::RemoveAt(o, f, rng.below(n)),
            6 => Op::SetAt(o, f, rng.below(n), v),
            7 => Op::Move(o, f, rng.below(n), rng.below(n)),
            8 => Op::SetList(
                o,
                f,
                (0..rng.below(3)).map(|_| random_value(&mut rng, meta, f, &objs)).collect(),
            ),
            _ => Op::Clear(o, f),
        });
    }
    ops
}

#[test]
fn pcm_scripts_equal_qvto_emf() {
    let Some(conf) = registry::pcm_conf() else { return };
    let base = Session::new();
    let ours = base.meta().clone();
    let mut rs0 = conf.resource_set_for(&registry::repo_root().join("x"));
    for (_, loc) in qvto_simoxide::registry::sim_top_packages() {
        rs0.load_metamodel(loc).unwrap();
    }
    let map = id_map(&ours, rs0.graph.meta());
    let sim_classes = simoxide_model::meta::CLASSES.len() as u32;
    let classes: Vec<ClassId> = ours
        .classes()
        .filter(|c| c.0 < sim_classes && !ours.class(*c).is_abstract && !ours.class(*c).is_interface)
        .collect();
    let focus: Vec<ClassId> = classes
        .iter()
        .copied()
        .filter(|c| FOCUS.contains(&&*ours.class(*c).name))
        .collect();
    assert!(focus.len() >= 20, "{}", focus.len());
    let mut total_ops = 0;
    let mut journal_len = 0;
    for seed in 0..SCRIPTS {
        let ops = script(seed, &ours, &classes, &focus);
        total_ops += ops.len();
        // the adapter, with a journal
        let mut session = Session::new();
        session.start_journal();
        let mut g = raw::Graph::default();
        let mut a = Side {
            s: SimStore::new(&mut g, &mut session),
            map: None,
            objs: Vec::new(),
            res: Vec::new(),
        };
        a.res = vec![a.s.create_resource("mem:/r1"), a.s.create_resource("mem:/r2")];
        // qvto-emf
        let mut b = Side {
            s: rs0.graph.clone(),
            map: Some(&map),
            objs: Vec::new(),
            res: Vec::new(),
        };
        b.res = vec![b.s.create_resource("mem:/r1"), b.s.create_resource("mem:/r2")];
        for (k, op) in ops.iter().enumerate() {
            let ra = a.apply(op);
            let rb = b.apply(op);
            assert_eq!(ra, rb, "script {seed} op {k} {op:?}");
        }
        let sa = a.state(&ours);
        let sb = b.state(&ours);
        if sa != sb {
            let line = sa.lines().zip(sb.lines()).find(|(x, y)| x != y);
            panic!("script {seed}: final states differ, first: {line:?}");
        }
        drop(a);
        let j: Journal = session.take_journal();
        journal_len += j.changes.len();
    }
    eprintln!("{SCRIPTS} scripts, {total_ops} operations, {journal_len} journal entries");
    let _ = ResourceSet::new();
}
