//! Differential test of the store against dynamic EMF: random scripts of model-API operations
//! (set, unset, add, insert, remove, move, clear, setEList, roots, delete, with wrong types and
//! bad indexes) on the contract metamodel, run here and by `oracle/run.sh scripts`. Every
//! operation's result and the final state (all features, `isSet`, containers, resources) must
//! be identical.
//!
//! `tests/data/scripts-oracle.tsv` holds the hashes of EMF's outputs. To regenerate:
//! `QVTO_EMF_WRITE_SCRIPTS=<dir> cargo test -p qvto-emf --test scripts`, then
//! `oracle/run.sh scripts /dev/null crates/qvto-model/src/contract.ecore <dir> <out>` and copy
//! `<out>/results.tsv`. With `QVTO_EMF_ORACLE_SCRIPTS=<out>` mismatches show the first
//! differing line.

use qvto_emf::canon::{first_difference, fnv64};
use qvto_emf::datatypes::java_double_to_string;
use qvto_emf::{Graph, ObjId};
use qvto_model::contract::{build_metamodel, Roles};
use qvto_model::{ClassId, Datum, FeatureId, ModelError, ModelStore, ResourceId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SCRIPTS: usize = 300;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

const SINGLE: [&str; 15] = [
    "name", "id", "count", "label", "opt", "kind", "big", "ratio", "flag", "boxed", "parent", "child", "friend", "owner", "only",
];
const MANY: [&str; 9] = ["tags", "nums", "children", "xchildren", "links", "bag", "owned", "targets", "sources"];

fn value_for(rng: &mut Rng, feature: &str, vars: &[String]) -> String {
    let obj = |rng: &mut Rng| format!("o:{}", vars[rng.below(vars.len())]);
    let ref_many = matches!(feature, "children" | "xchildren" | "links" | "bag" | "owned" | "targets" | "sources");
    if rng.chance(8) {
        // wrong type or null (dynamic EMF reference lists accept any Java object, so those only
        // get null)
        if ref_many {
            return "null".into();
        }
        return match rng.below(4) {
            0 => "null".into(),
            1 => "s:x".into(),
            2 => "i:1".into(),
            _ => obj(rng),
        };
    }
    match feature {
        "name" | "id" | "label" | "opt" | "tags" | "extra" => {
            let v = ["a", "b", "c", "x"][rng.below(4)];
            if rng.chance(10) { "null".into() } else { format!("s:{v}") }
        }
        "count" | "nums" => format!("i:{}", rng.below(3)),
        "boxed" => format!("i:{}", rng.below(2)),
        "big" => format!("l:{}", rng.below(3)),
        "ratio" => ["d:0.0", "d:1.5", "d:-0.0", "d:1.0E10"][rng.below(4)].into(),
        "flag" => ["b:true", "b:false"][rng.below(2)].into(),
        "kind" => ["e:A", "e:B", "e:C"][rng.below(3)].into(),
        _ => {
            if rng.chance(5) { "null".into() } else { obj(rng) }
        }
    }
}

fn generate(seed: u64) -> Vec<String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut lines = vec!["res r0 mem:/r0".to_string(), "res r1 mem:/r1".to_string()];
    let n = 4 + rng.below(4);
    let mut vars = Vec::new();
    for i in 0..n {
        let v = format!("n{i}");
        let class = if rng.chance(25) { "Sub" } else { "Node" };
        lines.push(format!("new {v} {class}"));
        vars.push(v);
    }
    if rng.chance(10) {
        lines.push("new zz Base".into());
    }
    let mut probe = Runner::new();
    for l in &lines {
        let a: Vec<&str> = l.split(' ').collect();
        let _ = probe.exec(&a);
    }
    let ops = 25 + rng.below(40);
    for _ in 0..ops {
        let v = vars[rng.below(vars.len())].clone();
        let r = format!("r{}", rng.below(2));
        let line = match rng.below(20) {
            0..=4 => {
                let f = if rng.chance(5) { "extra" } else { SINGLE[rng.below(SINGLE.len())] };
                format!("set {v} {f} {}", value_for(&mut rng, f, &vars))
            }
            5 => {
                let f = if rng.chance(50) { SINGLE[rng.below(SINGLE.len())] } else { MANY[rng.below(MANY.len())] };
                format!("unset {v} {f}")
            }
            6..=9 => {
                let f = MANY[rng.below(MANY.len())];
                let val = value_for(&mut rng, f, &vars);
                if rng.chance(30) {
                    format!("add {v} {f} {val} {}", rng.below(4))
                } else {
                    format!("add {v} {f} {val}")
                }
            }
            10 => {
                let f = MANY[rng.below(MANY.len())];
                format!("remove {v} {f} {}", value_for(&mut rng, f, &vars))
            }
            11 => format!("removeat {v} {} {}", MANY[rng.below(MANY.len())], rng.below(3)),
            12 => {
                let f = MANY[rng.below(MANY.len())];
                format!("setat {v} {f} {} {}", rng.below(3), value_for(&mut rng, f, &vars))
            }
            13 => format!("clear {v} {}", MANY[rng.below(MANY.len())]),
            14 => format!("move {v} {} {} {}", MANY[rng.below(MANY.len())], rng.below(3), rng.below(3)),
            15 => {
                let f = MANY[rng.below(MANY.len())];
                let k = rng.below(4);
                let vals: Vec<String> = (0..k)
                    .map(|_| {
                        let x = value_for(&mut rng, f, &vars);
                        if x == "null" && !matches!(f, "tags" | "nums") { format!("o:{}", vars[0]) } else { x }
                    })
                    .collect();
                format!("setlist {v} {f} {}", vals.join(" "))
            }
            16 | 17 => {
                if rng.chance(20) {
                    format!("root {r} {v} {}", rng.below(3))
                } else {
                    format!("root {r} {v}")
                }
            }
            18 => format!("unroot {r} {v}"),
            _ => {
                if rng.chance(30) {
                    format!("delete {v}")
                } else {
                    format!("set {v} {} {}", "friend", value_for(&mut rng, "friend", &vars))
                }
            }
        };
        let line = line.trim_end().to_string();
        // EMF does not prevent containment cycles (and then loops forever): skip such operations
        let mut next = probe.clone();
        let a: Vec<&str> = line.split(' ').collect();
        let _ = next.exec(&a);
        if !next.has_cycle() {
            probe = next;
            lines.push(line);
        }
    }
    lines
}

#[derive(Clone)]
struct Runner {
    g: Graph,
    roles: Roles,
    objs: HashMap<String, ObjId>,
    order: Vec<String>,
    names: HashMap<ObjId, String>,
    res: Vec<(String, ResourceId)>,
}

fn err(e: &ModelError) -> String {
    match e {
        ModelError::NoSuchFeature { .. } => "NoSuchFeature",
        ModelError::WrongMultiplicity(_) => "WrongMultiplicity",
        ModelError::TypeMismatch { .. } => "TypeMismatch",
        ModelError::IndexOutOfBounds { .. } => "IndexOutOfBounds",
        ModelError::Duplicate(_) => "Duplicate",
        ModelError::NullNotAllowed(_) => "NullNotAllowed",
        ModelError::Abstract(_) => "Abstract",
        ModelError::RecursiveContainment => "RecursiveContainment",
        ModelError::Unsupported(m) if m.contains("no duplicates") => "Duplicate",
        ModelError::Unsupported(_) => "Unsupported",
    }
    .to_string()
}

impl Runner {
    fn new() -> Runner {
        let mut g = Graph::new();
        let roles = build_metamodel(g.meta_mut());
        Runner { g, roles, objs: HashMap::new(), order: Vec::new(), names: HashMap::new(), res: Vec::new() }
    }

    fn class(&self, n: &str) -> ClassId {
        match n {
            "Base" => self.roles.base,
            "Sub" => self.roles.sub,
            _ => self.roles.node,
        }
    }

    fn feature(&self, o: ObjId, n: &str) -> Result<FeatureId, String> {
        self.g.meta().feature_by_name(self.g.class_of(o), n).ok_or_else(|| "err NoSuchFeature".to_string())
    }

    fn value<'a>(&self, s: &'a str) -> Datum<'a, ObjId> {
        if s == "null" {
            return Datum::Null;
        }
        let v = &s[2..];
        match &s[..1] {
            "s" => Datum::Str(v),
            "i" => Datum::Int(v.parse().unwrap()),
            "l" => Datum::Long(v.parse().unwrap()),
            "d" => Datum::Double(v.parse().unwrap()),
            "b" => Datum::Bool(v == "true"),
            "e" => Datum::Enum(self.roles.kind, ["A", "B", "C"].iter().position(|x| *x == v).unwrap() as u32),
            "o" => Datum::Obj(self.objs[v]),
            _ => panic!("{s}"),
        }
    }

    fn many(&self, f: FeatureId) -> Result<(), String> {
        if self.g.meta().feature(f).is_many() { Ok(()) } else { Err("err WrongMultiplicity".into()) }
    }

    fn exec(&mut self, a: &[&str]) -> Result<String, String> {
        let r = |x: Result<(), ModelError>| x.map(|_| "ok".to_string()).map_err(|e| format!("err {}", err(&e)));
        let rb = |x: Result<bool, ModelError>| x.map(|b| format!("ok {b}")).map_err(|e| format!("err {}", err(&e)));
        match a[0] {
            "new" => {
                let c = self.class(a[2]);
                let o = self.g.create(c).map_err(|e| format!("err {}", err(&e)))?;
                self.objs.insert(a[1].into(), o);
                self.names.insert(o, a[1].into());
                self.order.push(a[1].into());
                return Ok("ok".into());
            }
            "res" => {
                let id = self.g.create_resource(a[2]);
                self.res.push((a[1].into(), id));
                return Ok("ok".into());
            }
            "root" | "unroot" => {
                let rid = self.res.iter().find(|(n, _)| n == a[1]).unwrap().1;
                let o = self.objs[a[2]];
                if a[0] == "unroot" {
                    return Ok(format!("ok {}", self.g.remove_root(rid, o)));
                }
                return match a.get(3) {
                    Some(i) => r(self.g.add_root(rid, o, Some(i.parse().unwrap())).map(|_| ())),
                    None => rb(self.g.add_root(rid, o, None)),
                };
            }
            "delete" => {
                let o = self.objs[a[1]];
                self.g.delete(o);
                return Ok("ok".into());
            }
            _ => {}
        }
        let o = self.objs[a[1]];
        let f = self.feature(o, a[2])?;
        match a[0] {
            "set" => {
                if self.g.meta().feature(f).is_many() {
                    return Err("err WrongMultiplicity".into());
                }
                let v = self.value(a[3]);
                r(self.g.set(o, f, v))
            }
            "unset" => r(self.g.unset(o, f)),
            "add" => {
                self.many(f)?;
                let v = self.value(a[3]);
                match a.get(4) {
                    Some(i) => r(self.g.add(o, f, v, Some(i.parse().unwrap())).map(|_| ())),
                    None => rb(self.g.add(o, f, v, None)),
                }
            }
            "remove" => {
                self.many(f)?;
                let v = self.value(a[3]);
                rb(self.g.remove(o, f, v))
            }
            "removeat" => {
                self.many(f)?;
                r(self.g.remove_at(o, f, a[3].parse().unwrap()))
            }
            "setat" => {
                self.many(f)?;
                let v = self.value(a[4]);
                r(self.g.set_at(o, f, a[3].parse().unwrap(), v))
            }
            "clear" => {
                self.many(f)?;
                r(self.g.clear(o, f))
            }
            "move" => {
                self.many(f)?;
                r(self.g.move_to(o, f, a[3].parse().unwrap(), a[4].parse().unwrap()))
            }
            "setlist" => {
                self.many(f)?;
                let vals: Vec<Datum<'_, ObjId>> = a[3..].iter().map(|s| self.value(s)).collect();
                let vals: Vec<qvto_model::OwnedDatum<ObjId>> = vals.iter().map(|d| d.to_owned()).collect();
                let refs: Vec<Datum<'_, ObjId>> = vals.iter().map(|d| d.as_datum()).collect();
                r(self.g.set_list(o, f, &refs))
            }
            x => panic!("{x}"),
        }
    }

    fn has_cycle(&self) -> bool {
        self.order.iter().any(|v| {
            let mut cur = self.objs[v];
            for _ in 0..=self.order.len() {
                match self.g.container(cur) {
                    Some((p, _)) => cur = p,
                    None => return false,
                }
            }
            true
        })
    }

    fn show(&self, d: Datum<'_, ObjId>) -> String {
        match d {
            Datum::Null => "null".into(),
            Datum::Obj(o) => self.names.get(&o).cloned().unwrap_or("?".into()),
            Datum::Str(s) | Datum::Other(s) => s.into(),
            Datum::Double(x) => java_double_to_string(x),
            Datum::Enum(e, i) => self.g.meta().enumeration(e).literals[i as usize].literal.to_string(),
            Datum::Bool(b) => b.to_string(),
            d => d.as_i64().map(|i| i.to_string()).unwrap_or_default(),
        }
    }

    fn state(&self, out: &mut String) {
        out.push_str("--\n");
        let rname = |r: Option<ResourceId>| {
            r.and_then(|r| self.res.iter().find(|(_, x)| *x == r)).map_or("-".to_string(), |(n, _)| n.clone())
        };
        for (n, r) in &self.res {
            let roots: Vec<String> = self.g.roots(*r).iter().map(|o| self.show(Datum::Obj(*o))).collect();
            out.push_str(&format!("@{n} [{}]\n", roots.join(",")));
        }
        for v in &self.order {
            let o = self.objs[v];
            let class = self.g.meta().class(self.g.class_of(o));
            let c = match self.g.container(o) {
                Some((p, f)) => format!("{}.{}", self.show(Datum::Obj(p)), self.g.meta().feature(f).name),
                None => "-".into(),
            };
            out.push_str(&format!(
                "#{v} {} c={c} r={} dr={}\n",
                class.name,
                rname(self.g.resource_of(o)),
                rname(self.g.direct_resource(o))
            ));
            for &f in class.all_features.iter() {
                let fi = self.g.meta().feature(f);
                let set = if self.g.is_set(o, f) { "*" } else { "" };
                let val = if fi.is_many() {
                    let items: Vec<String> = (0..self.g.len(o, f)).map(|i| self.show(self.g.get_at(o, f, i))).collect();
                    format!("[{}]", items.join(","))
                } else {
                    self.show(self.g.get(o, f))
                };
                out.push_str(&format!("  {}{set}={val}\n", fi.name));
            }
        }
    }
}

fn run(lines: &[String]) -> String {
    let mut rn = Runner::new();
    let mut out = String::new();
    for l in lines {
        let a: Vec<&str> = l.split(' ').collect();
        let res = match rn.exec(&a) {
            Ok(s) | Err(s) => s,
        };
        out.push_str(&format!("{l} => {res}\n"));
    }
    rn.state(&mut out);
    out
}

#[test]
fn scripts_match_emf() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    if let Ok(dir) = std::env::var("QVTO_EMF_WRITE_SCRIPTS") {
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..SCRIPTS {
            let text = generate(i as u64 + 1).join("\n") + "\n";
            std::fs::write(Path::new(&dir).join(format!("{i:04}.script")), text).unwrap();
        }
    }
    let expected = std::fs::read_to_string(data.join("scripts-oracle.tsv")).unwrap();
    let oracle: Option<PathBuf> = std::env::var("QVTO_EMF_ORACLE_SCRIPTS").ok().map(PathBuf::from);
    let mut failures = Vec::new();
    for (i, line) in expected.lines().enumerate() {
        let (_, hash) = line.split_once('\t').unwrap();
        let out = run(&generate(i as u64 + 1));
        if fnv64(out.as_bytes()) != hash {
            let mut msg = format!("script {i:04} differs");
            if let Some(dir) = &oracle
                && let Ok(exp) = std::fs::read_to_string(dir.join(format!("{i:04}.out")))
                && let Some((l, a, b)) = first_difference(&exp, &out)
            {
                msg.push_str(&format!(" at line {l}:\n  EMF:  {a}\n  ours: {b}"));
            }
            failures.push(msg);
        }
    }
    eprintln!("{} scripts, {} differ", expected.lines().count(), failures.len());
    for f in failures.iter().take(30) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty());
}
