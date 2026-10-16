//! `qvto fuzz robust`: robustness fuzzing of the whole pipeline (parser, analyser, bytecode
//! compiler, VM) on arbitrary input, without an oracle: no panics, no hangs, no aborts, errors
//! reported. cargo-fuzz needs a nightly toolchain, which this machine does not have; this is a
//! mutational fuzzer of our own (PLAN Q7):
//!
//! - seeds: every `.qvto` file of the Eclipse suite and the SimuLizar/SPD corpus, and freshly
//!   generated expression and program cases (`qvto_testkit::fuzz`);
//! - mutations on a character-class tokenisation: deleting, duplicating, swapping and replacing
//!   token ranges, inserting keywords and punctuation (from `qvto_syntax`'s generated tables) or
//!   tokens of other seeds, splicing two seeds, numeric edge values, nesting bombs, non-ASCII
//!   characters; imports resolve to other mutated units (cycles included);
//! - every input is parsed, analysed against Ecore, the fuzz metamodel and the metamodels it
//!   names, compiled, and run (compat mode, a step budget) when it compiles without errors.
//!
//! Workers are child processes (`qvto fuzz robust-worker`) under an address-space limit; the
//! parent restarts a worker that dies (abort, stack overflow, out of memory) or stops making
//! progress (hang), and keeps the input it was working on. Panics are caught in the worker.
//! Everything found goes to `<out>/crashes/`.
//!
//! ```text
//! qvto fuzz robust [--workers 4] [--minutes 30] [--seed N] [--out DIR] [--mem-mb 3000]
//! qvto fuzz robust-replay FILE.qvto...      (runs the pipeline on files, reports panics)
//! ```

use crate::engine;
use qvto_model::ModelStore;
use qvto_sema::{UnitResolver, UnitSource};
use qvto_testkit::fuzz::{self as fz, Rng};
use qvto_testkit::paths;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

struct Opts {
    workers: usize,
    minutes: f64,
    seed: u64,
    out: PathBuf,
    mem_mb: u64,
    id: usize,
}

fn parse(root: &Path, args: &[String]) -> Opts {
    let mut o = Opts {
        workers: 4,
        minutes: 30.0,
        seed: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1),
        out: root.join("target/qvto-fuzz/robust"),
        mem_mb: 3000,
        id: 0,
    };
    let mut i = 0;
    let val = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i).cloned().unwrap_or_else(|| crate::die("missing value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--workers" => o.workers = val(&mut i).parse().unwrap_or(4).min(4),
            "--minutes" => o.minutes = val(&mut i).parse().unwrap_or(30.0),
            "--seed" => o.seed = val(&mut i).parse().unwrap_or(1),
            "--out" => o.out = val(&mut i).into(),
            "--mem-mb" => o.mem_mb = val(&mut i).parse().unwrap_or(3000),
            "--id" => o.id = val(&mut i).parse().unwrap_or(0),
            a => crate::die(&format!("fuzz robust: unknown argument {a}")),
        }
        i += 1;
    }
    o
}

/// The parent: runs the workers for the given time, restarting dead and stuck ones.
pub fn main(root: &Path, args: &[String]) {
    let o = parse(root, args);
    let crashes = o.out.join("crashes");
    std::fs::create_dir_all(&crashes).unwrap_or_else(|e| crate::die(&e.to_string()));
    let exe = std::env::current_exe().unwrap_or_else(|e| crate::die(&e.to_string()));
    let deadline = Instant::now() + Duration::from_secs_f64(o.minutes * 60.0);
    let spawn = |k: usize, generation: u64| -> Child {
        let minutes = (deadline.saturating_duration_since(Instant::now()).as_secs_f64() / 60.0).max(0.05);
        // an address-space limit, so that runaway allocation aborts the worker instead of the machine
        let cmd = format!(
            "ulimit -v {}; exec nice -n 19 \"{}\" fuzz robust-worker --id {k} --seed {} --minutes {minutes} --out \"{}\"",
            o.mem_mb * 1024,
            exe.display(),
            o.seed.wrapping_mul(1000).wrapping_add(k as u64 * 100 + generation),
            o.out.display()
        );
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| crate::die(&e.to_string()))
    };
    let mut gens = vec![0u64; o.workers];
    let mut kids: Vec<Option<Child>> = (0..o.workers).map(|k| Some(spawn(k, 0))).collect();
    let mut found = 0usize;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(2));
        for k in 0..o.workers {
            let cur = o.out.join(format!("w{k}.cur"));
            let stale = std::fs::metadata(&cur)
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(60))
                .unwrap_or(false);
            let child = kids[k].as_mut().unwrap();
            let status = child.try_wait().ok().flatten();
            let what = match (status, stale) {
                (Some(s), _) if s.success() => None,
                (Some(s), _) => Some(format!("died: {s}")),
                (None, true) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Some("hang (no progress for 60 s)".to_string())
                }
                (None, false) => continue,
            };
            if let Some(what) = what {
                let mut err = String::new();
                if let Some(e) = child.stderr.as_mut() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                found += 1;
                let name = format!("w{k}-{}-{}", gens[k], found);
                let _ = std::fs::copy(&cur, crashes.join(format!("{name}.qvto")));
                for x in 0..4 {
                    let _ = std::fs::copy(o.out.join(format!("w{k}.unit{x}")), crashes.join(format!("{name}.unit{x}")));
                }
                let tail: String = err.lines().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                let _ = std::fs::write(crashes.join(format!("{name}.txt")), format!("{what}\n{tail}\n"));
                eprintln!("worker {k}: {what}; input kept as crashes/{name}.qvto");
            }
            if Instant::now() < deadline {
                gens[k] += 1;
                kids[k] = Some(spawn(k, gens[k]));
            }
        }
    }
    for k in kids.iter_mut().flatten() {
        let _ = k.wait();
    }
    let mut total = 0u64;
    for k in 0..o.workers {
        if let Ok(t) = std::fs::read_to_string(o.out.join(format!("w{k}.stats"))) {
            println!("worker {k}: {}", t.trim());
            total += t.split_whitespace().nth(1).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
        }
    }
    println!("{total} inputs (last worker generations), {found} worker deaths/hangs; panics in {}", crashes.display());
}

/// Seeds: the corpus files.
fn seeds(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    for d in ["reference/eclipse-qvto/tests", "crates/qvto-testkit/corpus", "reference/external"] {
        walk(&root.join(d), &mut v);
    }
    v
}

fn walk(d: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    let mut es: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    es.sort();
    for p in es {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "qvto")
            && let Ok(b) = std::fs::read(&p)
        {
            out.push(String::from_utf8_lossy(&b).into_owned());
        }
    }
}

/// Character-class tokens: identifier/number runs, whitespace runs, single other characters.
fn tokens(s: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut start = 0;
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut prev: Option<u8> = None;
    for (i, c) in s.char_indices() {
        let k = class(c);
        if prev.is_some_and(|p| p != k || k == 2) && i > start {
            v.push(&s[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < s.len() {
        v.push(&s[start..]);
    }
    v
}

fn dictionary() -> Vec<String> {
    let mut d: Vec<String> = qvto_syntax::TokenKind::ALL
        .iter()
        .filter_map(|k| k.spelling())
        .map(|s| format!(" {s} "))
        .collect();
    for s in [
        " 2147483648 ", " 9223372036854775807 ", " 99999999999999999999 ", " 1.0E400 ", " -0.0 ",
        " 'x' ", " '\\u0041' ", " '\\4a' ", " \"q\" ", " null ", " invalid ", " self ", " result ",
        " this ", " main ", " Set{1} ", " Sequence{1..1000000} ", " Dict{1=2} ", " List{} ",
        " Tuple{a=1} ", " ->closure(x | x) ", " .oclAsType(OclAny) ", " ->iterate(a; b = 0 | b) ",
        " xmap ", " resolveone ", " late resolveIn(", " \u{b4} ", " \u{feff} ", " \u{0} ",
        " \u{1F600} ", " /* ", " */ ", " -- ", "\r", " _ ", " :: ", " # ", " ` ",
    ] {
        d.push(s.to_string());
    }
    d
}

/// One mutated input.
fn mutate(rng: &mut Rng, seeds: &[String], dict: &[String], base: &str) -> String {
    let mut toks: Vec<String> = tokens(base).into_iter().map(String::from).collect();
    let n = 1 + rng.below(4);
    for _ in 0..n {
        if toks.is_empty() {
            toks.push(rng.pick(dict).clone());
        }
        let len = toks.len();
        let i = rng.below(len);
        let span = (1 + rng.below(3)).min(len - i);
        match rng.below(11) {
            0 => {
                toks.drain(i..i + span);
            }
            1 => {
                let dup: Vec<String> = toks[i..i + span].to_vec();
                for (k, t) in dup.into_iter().enumerate() {
                    toks.insert(i + span + k, t);
                }
            }
            2 => {
                let j = rng.below(len);
                toks.swap(i, j);
            }
            3 | 4 => {
                toks.insert(i, rng.pick(dict).clone());
            }
            5 => {
                toks[i] = rng.pick(dict).clone();
            }
            6 => {
                let other = rng.pick(seeds);
                let ot = tokens(other);
                if !ot.is_empty() {
                    let j = rng.below(ot.len());
                    let k = (j + 1 + rng.below(20)).min(ot.len());
                    for (m, t) in ot[j..k].iter().enumerate() {
                        toks.insert(i + m, t.to_string());
                    }
                }
            }
            7 => {
                // splice: our head, another seed's tail
                let other = rng.pick(seeds);
                let ot = tokens(other);
                if !ot.is_empty() {
                    let j = rng.below(ot.len());
                    toks.truncate(i);
                    toks.extend(ot[j..].iter().map(|t| t.to_string()));
                }
            }
            8 => {
                // numeric edge values
                if toks[i].bytes().all(|b| b.is_ascii_digit()) {
                    toks[i] = rng
                        .pick(&["0", "2147483648", "9223372036854775807", "99999999999999999999", "1000000000"])
                        .to_string();
                }
            }
            9 => {
                // a nesting bomb
                let depth = *rng.pick(&[50usize, 200, 300, 1000, 5000]);
                let (open, close) = *rng.pick(&[
                    ("(", ")"),
                    ("Sequence{", "}"),
                    ("if true then ", " else 0 endif"),
                    ("not ", ""),
                    ("-", ""),
                    ("do { ", " }"),
                    ("Tuple{a=", "}"),
                ]);
                toks.insert(i, format!("{}1{}", open.repeat(depth), close.repeat(depth)));
            }
            _ => {
                // a character from the edges of Unicode and the lexer's quirks
                let c = *rng.pick(&['\u{0}', '\u{b4}', '\u{ffff}', '\u{fffd}', '\u{1F600}', '\u{7f}', '\r', '\u{2028}', '\\', '\'', '"']);
                toks.insert(i, c.to_string());
            }
        }
    }
    toks.concat()
}

/// Imports resolve to the given units by a hash of the name (so cycles happen).
struct MemResolver {
    units: Vec<String>,
}

impl UnitResolver for MemResolver {
    fn resolve(&self, _importer: &UnitSource, qualified_name: &str) -> Option<UnitSource> {
        if self.units.is_empty() {
            return None;
        }
        let h = qualified_name.bytes().fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
        let k = (h % (self.units.len() as u64 + 1)) as usize;
        let text = self.units.get(k)?;
        Some(unit(&format!("mem{k}"), qualified_name, text))
    }
}

fn unit(key: &str, qn: &str, text: &str) -> UnitSource {
    UnitSource {
        key: key.to_string(),
        qualified_name: qn.to_string(),
        file_name: format!("{}.qvto", qn.rsplit('.').next().unwrap_or(qn)),
        path: None,
        text: Rc::from(text),
        blackbox: None,
    }
}

/// The pipeline on one input: parse, analyse, compile, run. Returns a one-word summary.
fn check(ctx: &Ctx, text: &str, imports: &[String], model: &str) -> &'static str {
    let parsed = qvto_syntax::parse(text);
    let _ = qvto_syntax::print::unit(&parsed.unit);
    let mem = MemResolver {
        units: imports.to_vec(),
    };
    let resolver = engine::cli_resolver_dyn(&mem, &ctx.catalog);
    let main = unit("main", "fuzzmain", text);
    let uris = qvto_sema::scan_metamodel_uris(&main, &resolver);
    let mut rs = ctx.models.resource_set_for(&ctx.dir.join("main.qvto"));
    let _ = rs.load(&ctx.fuzz_ecore_uri);
    for uri in &uris {
        let _ = engine::load_metamodel(&mut rs, uri);
    }
    rs.ecore_resource();
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let ders = qvto_emf::operations::meta_derivations(&rs.graph);
    let c = qvto::compile_with_metamodel(main, &resolver, rs.graph.meta().clone(), &ops, &ders);
    if c.has_errors() {
        return "errors";
    }
    let Some(p) = c.program else { return "no-program" };
    // input: the fuzz model for every in/inout parameter
    let _ = std::fs::write(ctx.dir.join("in.xmi"), model);
    let mut a = Vec::new();
    let main_mod = &p.ir.modules[p.ir.main_module.index()];
    let input = engine::get_or_load(&mut rs, &ctx.dir.join("in.xmi")).ok();
    for mp in &main_mod.model_params {
        match (mp.direction, input) {
            (qvto_sema::ir::Direction::Out, _) | (_, None) => a.push(Vec::new()),
            (_, Some(r)) => a.push(rs.graph.roots(r).to_vec()),
        }
    }
    rs.resolve_all();
    let opts = qvto::RunOptions {
        step_limit: Some(2_000_000),
        ..qvto::RunOptions::default()
    };
    let mut vm = qvto::Vm::new(&p, &mut rs.graph, opts);
    vm.set_natives(Rc::new(crate::bbox::natives()));
    let out = vm.run_transformation(a);
    let _ = vm.log.len();
    if out.severity == 0 { "ran" } else { "ran-error" }
}

struct Ctx {
    models: qvto_emf::config::Config,
    catalog: qvto::blackbox::Catalog,
    dir: PathBuf,
    fuzz_ecore_uri: String,
}

/// A worker: mutates and checks inputs until its time is up.
pub fn worker(root: &Path, args: &[String]) {
    let o = parse(root, args);
    let dir = o.out.join(format!("w{}", o.id));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| crate::die(&e.to_string()));
    let _ = std::fs::write(dir.join("fuzz.ecore"), fz::FUZZ_ECORE);
    let ctx = Ctx {
        models: qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), root)
            .unwrap_or_else(|e| crate::die(&e.to_string())),
        catalog: engine::cli_catalog(root),
        fuzz_ecore_uri: qvto_emf::uri::Uri::file(&std::path::absolute(dir.join("fuzz.ecore")).unwrap().to_string_lossy()).to_string(),
        dir: dir.clone(),
    };
    let seeds = seeds(root);
    let dict = dictionary();
    let mut rng = Rng::new(o.seed);
    let deadline = Instant::now() + Duration::from_secs_f64(o.minutes * 60.0);
    let cur = o.out.join(format!("w{}.cur", o.id));
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut n = 0u64;
    let crashes = o.out.join("crashes");
    let _ = std::fs::create_dir_all(&crashes);
    std::panic::set_hook(Box::new(|_| {}));
    while Instant::now() < deadline {
        n += 1;
        // a base: a corpus seed or a generated case
        let base = match rng.below(10) {
            0 => {
                let mut r = rng.fork();
                let stmts: Vec<_> = {
                    let mut g = fz::expr::Gen::new(&mut r, fz::expr::base_scope());
                    (0..8).map(|k| fz::expr::statement(&mut g, &format!("@e{k}@"), 4)).collect()
                };
                fz::expr::transformation("g", &stmts)
            }
            1 => {
                let mut r = rng.fork();
                fz::prog::transformation(&mut r, "p")
            }
            _ => rng.pick(&seeds).clone(),
        };
        let text = if rng.pct(10) { base } else { mutate(&mut rng, &seeds, &dict, &base) };
        let nimp = if rng.pct(30) { 1 + rng.below(2) } else { 0 };
        let imports: Vec<String> = (0..nimp)
            .map(|_| {
                let b = rng.pick(&seeds).clone();
                if rng.pct(50) { mutate(&mut rng, &seeds, &dict, &b) } else { b }
            })
            .collect();
        let model = fz::model::generate(&mut rng);
        // the current input, for the parent if this process dies
        let _ = std::fs::write(&cur, &text);
        for (k, u) in imports.iter().enumerate() {
            let _ = std::fs::write(o.out.join(format!("w{}.unit{k}", o.id)), u);
        }
        for k in imports.len()..4 {
            let _ = std::fs::remove_file(o.out.join(format!("w{}.unit{k}", o.id)));
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&ctx, &text, &imports, &model)));
        match r {
            Ok(s) => *counts.entry(s).or_default() += 1,
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                *counts.entry("panic").or_default() += 1;
                let name = format!("panic-w{}-{n}", o.id);
                let _ = std::fs::write(crashes.join(format!("{name}.qvto")), &text);
                for (k, u) in imports.iter().enumerate() {
                    let _ = std::fs::write(crashes.join(format!("{name}.unit{k}")), u);
                }
                let _ = std::fs::write(crashes.join(format!("{name}.txt")), &msg);
            }
        }
        if n % 200 == 0 {
            let _ = std::fs::write(o.out.join(format!("w{}.stats", o.id)), format!("inputs {n} {counts:?}\n"));
        }
    }
    let _ = std::fs::write(o.out.join(format!("w{}.stats", o.id)), format!("inputs {n} {counts:?}\n"));
    let _ = std::io::stderr().flush();
}

/// Runs the pipeline on files (with `<file>.unitN` siblings as imports) and reports panics.
pub fn replay(root: &Path, files: &[String]) {
    let dir = root.join("target/qvto-fuzz/replay");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("fuzz.ecore"), fz::FUZZ_ECORE);
    let ctx = Ctx {
        models: qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), root)
            .unwrap_or_else(|e| crate::die(&e.to_string())),
        catalog: engine::cli_catalog(root),
        fuzz_ecore_uri: qvto_emf::uri::Uri::file(&std::path::absolute(dir.join("fuzz.ecore")).unwrap().to_string_lossy()).to_string(),
        dir: dir.clone(),
    };
    let model = fz::model::generate(&mut Rng::new(1));
    for f in files {
        let text = String::from_utf8_lossy(&std::fs::read(f).unwrap_or_default()).into_owned();
        let stem = f.strip_suffix(".qvto").unwrap_or(f);
        let imports: Vec<String> = (0..4)
            .filter_map(|k| std::fs::read(format!("{stem}.unit{k}")).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect();
        let t0 = Instant::now();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&ctx, &text, &imports, &model)));
        match r {
            Ok(s) => println!("{f}: {s} ({:.1} ms)", t0.elapsed().as_secs_f64() * 1e3),
            Err(_) => println!("{f}: PANIC"),
        }
    }
}
