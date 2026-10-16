//! Shared code of the golden and differential tests: the request language of the Java oracle
//! (`tools/oracle/Oracle.java`), its evaluation on the Rust side, and the oracle process.
#![allow(dead_code)]

use qvto_runtime::coll::ops;
use qvto_runtime::dict::Dict;
use qvto_runtime::eq::{java_equals, java_hash, ocl_equal};
use qvto_runtime::identity::{HashEnv, IdentityKey, Mode};
use qvto_runtime::{Coll, CollKind, IntBox, ObjRef, Str, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

pub mod generate;

/// Compat environment whose object identity hashes come from the requests (`o<id>.<hash>`).
pub struct TapeEnv {
    pub hashes: RefCell<HashMap<ObjRef, i32>>,
    pub mode: Mode,
}

impl TapeEnv {
    pub fn new(mode: Mode) -> Self {
        TapeEnv { hashes: RefCell::new(HashMap::new()), mode }
    }
}

impl HashEnv for TapeEnv {
    fn mode(&self) -> Mode {
        self.mode
    }
    fn identity_hash(&self, key: IdentityKey) -> i32 {
        match key {
            IdentityKey::Obj(o) => *self.hashes.borrow().get(&o).expect("object hash on tape"),
            IdentityKey::Cell(c) => c as i32,
            _ => 0,
        }
    }
    fn class_name(&self, key: IdentityKey) -> &str {
        match key {
            IdentityKey::Obj(_) => "Oracle$IdObj",
            k => qvto_runtime::identity::default_class_name(k),
        }
    }
}

pub fn hex16(units: impl IntoIterator<Item = u16>) -> String {
    units.into_iter().map(|u| format!("{u:04x}")).collect()
}

pub fn unhex16(h: &str) -> Vec<u16> {
    (0..h.len() / 4).map(|i| u16::from_str_radix(&h[4 * i..4 * i + 4], 16).unwrap()).collect()
}

pub fn decode(t: &str, env: &TapeEnv, regs: &[Value]) -> Value {
    let (k, r) = t.split_at(1);
    match k {
        "n" => Value::Null,
        "s" => Value::Str(Str::from_utf16(&unhex16(r))),
        "i" => Value::Int(r.parse::<i32>().unwrap() as i64),
        "l" => Value::long(r.parse().unwrap()),
        "d" => Value::Real(f64::from_bits(u64::from_str_radix(r, 16).unwrap())),
        "f" => Value::Float(f32::from_bits(u32::from_str_radix(r, 16).unwrap())),
        "b" => Value::Bool(r == "1"),
        "h" => Value::Boxed(IntBox::Short, r.parse::<i16>().unwrap() as i32),
        "y" => Value::Boxed(IntBox::Byte, r.parse::<i8>().unwrap() as i32),
        "c" => Value::Boxed(IntBox::Char, u16::from_str_radix(r, 16).unwrap() as i32),
        "o" => {
            let (id, h) = r.split_once('.').unwrap();
            let o = ObjRef(id.parse().unwrap());
            env.hashes.borrow_mut().entry(o).or_insert(h.parse().unwrap());
            Value::Obj(o)
        }
        "r" => regs[r.parse::<usize>().unwrap()].clone(),
        _ => panic!("bad value {t}"),
    }
}

pub fn encode(v: &Value) -> String {
    match v {
        Value::Null => "n".into(),
        Value::Invalid => "X".into(),
        Value::Str(s) => format!("s{}", hex16(s.units())),
        Value::Int(i) => {
            if *i as i32 as i64 == *i {
                format!("i{i}")
            } else {
                format!("l{i}")
            }
        }
        Value::Boxed(IntBox::Long, i) => format!("l{i}"),
        Value::Boxed(IntBox::Short, i) => format!("h{i}"),
        Value::Boxed(IntBox::Byte, i) => format!("y{i}"),
        Value::Boxed(IntBox::Char, i) => format!("c{:04x}", *i as u16),
        Value::Real(d) => format!("d{:016x}", d.to_bits()),
        Value::Float(f) => format!("f{:08x}", f.to_bits()),
        Value::Bool(b) => if *b { "b1" } else { "b0" }.into(),
        Value::Obj(o) => format!("o{}", o.0),
        Value::Coll(c) => {
            let k = match c.kind() {
                CollKind::Set => "S",
                CollKind::OrderedSet => "O",
                CollKind::Bag => "B",
                CollKind::Sequence => "Q",
            };
            let items: Vec<String> = c.iter().map(encode).collect();
            format!("{k}[{}]", items.join(","))
        }
        Value::List(l) => {
            let items: Vec<String> = l.borrow().items.iter().map(encode).collect();
            format!("Q[{}]", items.join(","))
        }
        Value::Dict(d) => {
            let items: Vec<String> = d.borrow().iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
            format!("D[{}]", items.join(","))
        }
        other => format!("?{other:?}"),
    }
}

fn kind_of(k: &str) -> CollKind {
    match k {
        "S" => CollKind::Set,
        "O" => CollKind::OrderedSet,
        "B" => CollKind::Bag,
        "Q" => CollKind::Sequence,
        _ => panic!("kind {k}"),
    }
}

fn coll_mut(v: &mut Value) -> &mut Coll {
    match v {
        Value::Coll(c) => Rc::make_mut(c),
        _ => panic!("not a collection"),
    }
}

fn coll_ref(v: &Value) -> Option<&Coll> {
    match v {
        Value::Coll(c) => Some(c),
        _ => None,
    }
}

/// Runs a collection script (see `Oracle.coll`).
pub fn run_coll(script: &str, env: &TapeEnv) -> String {
    let mut regs: Vec<Value> = vec![Value::Null; 32];
    let mut out: Vec<String> = Vec::new();
    for stmt in script.split(';') {
        let t: Vec<&str> = stmt.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        let r = |s: &str| s.parse::<usize>().unwrap();
        macro_rules! val {
            ($s:expr) => {
                decode($s, env, &regs)
            };
        }
        macro_rules! reg {
            ($s:expr) => {
                regs[r($s)].clone()
            };
        }
        let set = |regs: &mut Vec<Value>, dst: &str, v: Value| regs[r(dst)] = v;
        match t[0] {
            "new" => set(&mut regs, t[2], Coll::new_of(kind_of(t[1]), env).into()),
            "dict" => set(&mut regs, t[1], Value::Dict(Rc::new(RefCell::new(Dict::new(env))))),
            "+" => {
                let v = val!(t[2]);
                coll_mut(&mut regs[r(t[1])]).add(v, env);
            }
            "-" => {
                let v = val!(t[2]);
                coll_mut(&mut regs[r(t[1])]).remove(&v, env);
            }
            "clr" => coll_mut(&mut regs[r(t[1])]).clear(),
            "ret" => {
                let k: usize = t[2].parse().unwrap();
                let mut i = 0;
                if coll_mut(&mut regs[r(t[1])])
                    .retain_iter(
                        |_| {
                            i += 1;
                            i % k != 0
                        },
                        env,
                    )
                    .is_err()
                {
                    out.push("X".into());
                }
            }
            "rall" => {
                let other = reg!(t[2]);
                if coll_mut(&mut regs[r(t[1])]).remove_all(coll_ref(&other).unwrap(), env).is_err() {
                    out.push("X".into());
                }
            }
            "addall" => {
                let other = reg!(t[2]);
                coll_mut(&mut regs[r(t[1])]).add_all(coll_ref(&other).unwrap(), env);
            }
            "cp" => {
                let src = reg!(t[3]);
                let v = match coll_ref(&src) {
                    Some(c) => Coll::copy_of(kind_of(t[1]), c, env).into(),
                    None => Value::Invalid,
                };
                set(&mut regs, t[2], v)
            }
            "inc" => {
                let v = ops::including(reg!(t[2]), val!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "exc" => {
                let v = ops::excluding(reg!(t[2]), &val!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "qinc" => {
                let v = ops::qvto_including(reg!(t[2]), val!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "qexc" => {
                let v = ops::qvto_excluding(reg!(t[2]), &val!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "uni" => {
                let v = ops::union(reg!(t[2]), reg!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "int" => {
                let v = ops::intersection(reg!(t[2]), reg!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "min" => {
                let v = ops::qvto_minus(reg!(t[2]), reg!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "sym" => {
                let v = ops::qvto_symmetric_difference(reg!(t[2]), reg!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "fla" => {
                let v = ops::flatten(reg!(t[2]), env);
                set(&mut regs, t[1], v)
            }
            "qfla" => {
                let v = ops::qvto_flatten(reg!(t[2]), env);
                set(&mut regs, t[1], v)
            }
            "aset" => {
                let v = ops::as_set(reg!(t[2]), env);
                set(&mut regs, t[1], v)
            }
            "abag" => {
                let v = ops::as_bag(reg!(t[2]), env);
                set(&mut regs, t[1], v)
            }
            "aseq" => {
                let v = ops::as_sequence(reg!(t[2]), env);
                set(&mut regs, t[1], v)
            }
            "aos" => {
                let v = ops::as_ordered_set(reg!(t[2]), env);
                set(&mut regs, t[1], v)
            }
            "app" => {
                let v = ops::append(reg!(t[2]), val!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "pre" => {
                let v = ops::prepend(reg!(t[2]), val!(t[3]), env);
                set(&mut regs, t[1], v)
            }
            "ins" => {
                let v = ops::insert_at(reg!(t[2]), t[3].parse().unwrap(), val!(t[4]), env);
                set(&mut regs, t[1], v)
            }
            "subs" => {
                let v = ops::sub_sequence(reg!(t[2]), t[3].parse().unwrap(), t[4].parse().unwrap());
                set(&mut regs, t[1], v)
            }
            "subo" => {
                let v = ops::sub_ordered_set(reg!(t[2]), t[3].parse().unwrap(), t[4].parse().unwrap(), env);
                set(&mut regs, t[1], v)
            }
            "dput" => {
                let (k, v) = (val!(t[2]), val!(t[3]));
                match &regs[r(t[1])] {
                    Value::Dict(d) => {
                        d.borrow_mut().put(k, v, env);
                    }
                    _ => panic!(),
                }
            }
            "drem" => {
                let v = val!(t[2]);
                match &regs[r(t[1])] {
                    Value::Dict(d) => {
                        d.borrow_mut().remove_value(&v, env);
                    }
                    _ => panic!(),
                }
            }
            "dcp" => {
                let d = match &regs[r(t[2])] {
                    Value::Dict(d) => d.borrow().copy(env),
                    _ => panic!(),
                };
                set(&mut regs, t[1], Value::Dict(Rc::new(RefCell::new(d))))
            }
            "p" => out.push(encode(&regs[r(t[1])])),
            "ph" => out.push(format!("#{}", java_hash(&regs[r(t[1])], env))),
            "pin" | "pat" | "pidx" | "pcnt" | "psum" | "pmax" | "pmin" => {
                let Some(c) = coll_ref(&regs[r(t[1])]) else {
                    out.push("X".into());
                    continue;
                };
                let res = match t[0] {
                    "pin" => if c.contains(&val!(t[2]), env) { "T" } else { "F" }.to_string(),
                    "pat" => encode(&ops::at(c, t[2].parse().unwrap())),
                    "pidx" => encode(&ops::index_of(c, &val!(t[2]), env)),
                    "pcnt" => format!("i{}", ops::count(c, &val!(t[2]), env)),
                    "psum" => encode(&ops::sum(c).unwrap_or(Value::Null)),
                    "pmax" => encode(&ops::max(c)),
                    _ => encode(&ops::min(c)),
                };
                out.push(res);
            }
            "peq" => out.push(if ocl_equal(&regs[r(t[1])], &regs[r(t[2])], env) { "T" } else { "F" }.into()),
            "pjeq" => out.push(if java_equals(&regs[r(t[1])], &regs[r(t[2])], env) { "T" } else { "F" }.into()),
            other => panic!("statement {other}"),
        }
    }
    out.join(" ")
}

/// Evaluates one oracle request on the Rust side.
pub fn eval(line: &str) -> String {
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    match cmd {
        "hash" => {
            let env = TapeEnv::new(Mode::Compat);
            let v = decode(arg, &env, &[]);
            java_hash(&v, &env).to_string()
        }
        "coll" => run_coll(arg, &TapeEnv::new(Mode::Compat)),
        _ => extra::eval(cmd, arg),
    }
}

pub mod extra;

/// The Java oracle process.
pub struct Oracle {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

impl Oracle {
    pub fn start() -> Oracle {
        let script = crate_dir().join("tools/oracle/run.sh");
        let mut child = Command::new(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the Java oracle");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Oracle { child, stdin, stdout }
    }

    /// Answers for all requests (written by a thread while reading, so the pipes never block).
    pub fn ask_all(self, requests: &[String]) -> Vec<String> {
        let Oracle { mut child, mut stdin, mut stdout } = self;
        let reqs = requests.to_vec();
        let writer = std::thread::spawn(move || {
            let mut w = std::io::BufWriter::new(&mut stdin);
            for r in &reqs {
                writeln!(w, "{r}").unwrap();
            }
            w.flush().unwrap();
            drop(w);
            drop(stdin);
        });
        let mut out = Vec::with_capacity(requests.len());
        let mut line = String::new();
        for _ in 0..requests.len() {
            line.clear();
            stdout.read_line(&mut line).unwrap();
            out.push(line.trim_end_matches(['\n', '\r']).to_string());
        }
        writer.join().unwrap();
        child.wait().unwrap();
        out
    }
}

/// Golden file: `request \t expected` per line.
pub fn read_golden(path: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (a, b) = l.split_once('\t').expect("tab");
            (a.to_string(), b.to_string())
        })
        .collect()
}

pub fn write_golden(path: &Path, header: &str, pairs: &[(String, String)]) {
    let mut s = String::new();
    for h in header.lines() {
        s.push_str("# ");
        s.push_str(h);
        s.push('\n');
    }
    for (a, b) in pairs {
        s.push_str(a);
        s.push('\t');
        s.push_str(b);
        s.push('\n');
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, s).unwrap();
}
