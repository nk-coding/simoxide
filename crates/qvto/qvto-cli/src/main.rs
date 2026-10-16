//! `qvto`: the command line: `qvto conform` (the conformance scoreboard and the Java reference
//! runner's fixtures), `qvto run` (compile and run a transformation), `qvto disasm` (the
//! compiled program), `qvto bench` (run times of cases against the warm Java runner).

mod bbox;
mod engine;

use qvto_model::ModelStore;
use qvto_testkit::case::{Case, read_cases};
use qvto_testkit::engine::{Engine, StubEngine};
use qvto_testkit::fixture::{self, JavaResult};
use qvto_testkit::java::{JavaReplay, JavaRun, record};
use qvto_testkit::{find_root, paths, report};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "\
usage:
  qvto run FILE.qvto [MODEL...] [--repeat N] [--fast]
      Compiles and runs a transformation (models bound to the in/inout parameters in order,
      metamodels by namespace URI from the suite configuration); prints the log and outcome.
  qvto disasm FILE.qvto
      Prints the problems, modules, operations and the bytecode of a transformation.
  qvto bench [--suite eclipse|corpus] --only ID,... [--repeat N] [--java]
      Runs cases N times with the engine (and in one JVM of the reference runner) and prints
      compile and run times.
  qvto conform [--suite eclipse|corpus|all] [--cases FILE] [--fixture FILE]
               [--engine qvto|qvto-fast|stub|replay:DIR] [--only ID,...] [--prefix P]...
               [--report FILE.md] [--tsv FILE] [--verbose]
      Runs the engine on the cases and prints the scoreboard: per case the Eclipse
      driver's checks and the strict comparison with the Java runner's fixture.
  qvto conform record [--suite eclipse|corpus|all] [--only ID,...] [--prefix P]... [--runs N]
      Runs the Java reference runner N times (default 5, one JVM per run), marks
      outputs that differ between the runs as unstable and writes the fixture
      bundle; the full results (tapes, XMI) stay in target/qvto-testkit/java/<suite>.
  qvto conform java [--suite ...] [--out DIR] [--only ID,...] [--prefix P]... [--save-xmi]
      Runs the Java reference runner once into DIR.";

struct Suite {
    name: &'static str,
    cases: &'static str,
    fixture: &'static str,
}

const SUITES: [Suite; 2] = [
    Suite {
        name: "eclipse",
        cases: paths::ECLIPSE_CASES,
        fixture: paths::ECLIPSE_FIXTURE,
    },
    Suite {
        name: "corpus",
        cases: paths::CORPUS_CASES,
        fixture: paths::CORPUS_FIXTURE,
    },
];

fn die(msg: &str) -> ! {
    eprintln!("qvto: {msg}");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("conform") => conform(&args[1..]),
        Some("disasm") => disasm(&args[1..]),
        Some("run") => run_cmd(&args[1..]),
        Some("bench") => bench(&args[1..]),
        Some("-h" | "--help") | None => {
            println!("{USAGE}");
        }
        Some(c) => die(&format!(
            "unknown command {c}\n{USAGE}"
        )),
    }
}

#[derive(Default)]
struct Opts {
    sub: Option<String>,
    suite: Option<String>,
    cases: Option<PathBuf>,
    fixture: Option<PathBuf>,
    engine: Option<String>,
    only: Vec<String>,
    prefixes: Vec<String>,
    report: Option<PathBuf>,
    tsv: Option<PathBuf>,
    out: Option<PathBuf>,
    verbose: bool,
    save_xmi: bool,
    runs: usize,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        runs: 5,
        ..Opts::default()
    };
    let mut i = 0;
    let val = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i)
            .cloned()
            .unwrap_or_else(|| die(&format!("{} needs a value", args[*i - 1])))
    };
    while i < args.len() {
        match args[i].as_str() {
            "record" | "java" if i == 0 => o.sub = Some(args[i].clone()),
            "--suite" => o.suite = Some(val(&mut i)),
            "--cases" => o.cases = Some(val(&mut i).into()),
            "--fixture" => o.fixture = Some(val(&mut i).into()),
            "--engine" => o.engine = Some(val(&mut i)),
            "--only" => o.only.extend(val(&mut i).split(',').map(String::from)),
            "--prefix" => o.prefixes.push(val(&mut i)),
            "--report" => o.report = Some(val(&mut i).into()),
            "--tsv" => o.tsv = Some(val(&mut i).into()),
            "--out" => o.out = Some(val(&mut i).into()),
            "--verbose" | "-v" => o.verbose = true,
            "--save-xmi" => o.save_xmi = true,
            "--runs" => {
                o.runs = val(&mut i)
                    .parse()
                    .unwrap_or_else(|_| die("--runs needs a number"))
            }
            a => die(&format!("unknown argument {a}\n{USAGE}")),
        }
        i += 1;
    }
    o
}

/// The (name, case file, fixture) triples selected by the options.
fn suites(o: &Opts, root: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    if let Some(c) = &o.cases {
        let f = o
            .fixture
            .clone()
            .unwrap_or_else(|| c.with_extension("fixture.txt"));
        return vec![("custom".into(), c.clone(), f)];
    }
    let want = o.suite.as_deref().unwrap_or("all");
    SUITES
        .iter()
        .filter(|s| want == "all" || want == s.name)
        .filter(|s| root.join(s.cases).is_file())
        .map(|s| (s.name.to_string(), root.join(s.cases), root.join(s.fixture)))
        .collect()
}

fn conform(args: &[String]) {
    let o = parse(args);
    let root =
        find_root(Path::new(".")).unwrap_or_else(|| die("not inside the qvto-rs repository"));
    let suites = suites(&o, &root);
    if suites.is_empty() {
        die("no case files");
    }
    match o.sub.as_deref() {
        Some("record") => {
            for (name, cases, fixture) in &suites {
                let run = JavaRun {
                    base: root.clone(),
                    cases: cases.clone(),
                    out: root.join(paths::JAVA_CACHE).join(name),
                    only: o.only.clone(),
                    prefixes: o.prefixes.clone(),
                    tape: true,
                    save_xmi: true,
                    jvm_opts: None,
                };
                let _ = std::fs::remove_dir_all(&run.out);
                match record(&run, fixture, o.runs) {
                    Ok(log) => print!(
                        "{}",
                        log.lines()
                            .filter(|l| !l.starts_with("pass\t"))
                            .collect::<Vec<_>>()
                            .join("\n")
                            + "\n"
                    ),
                    Err(e) => die(&e),
                }
            }
        }
        Some("java") => {
            for (name, cases, _) in &suites {
                let run = JavaRun {
                    base: root.clone(),
                    cases: cases.clone(),
                    out: o
                        .out
                        .clone()
                        .map(|d| d.join(name))
                        .unwrap_or_else(|| root.join(paths::JAVA_CACHE).join(name)),
                    only: o.only.clone(),
                    prefixes: o.prefixes.clone(),
                    tape: true,
                    save_xmi: o.save_xmi,
                    jvm_opts: None,
                };
                match run.run() {
                    Ok(log) => print!("{log}"),
                    Err(e) => die(&e),
                }
            }
        }
        _ => scoreboard(&o, &root, &suites),
    }
}

fn scoreboard(o: &Opts, root: &Path, suites: &[(String, PathBuf, PathBuf)]) {
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), root)
        .unwrap_or_else(|e| die(&format!("{}: {e}", paths::SUITE_CONF)));
    let mut all = Vec::new();
    let mut engine_name = String::new();
    for (name, cases_file, fixture_file) in suites {
        let cases: Vec<Case> = read_cases(cases_file)
            .unwrap_or_else(|e| die(&format!("{}: {e}", cases_file.display())));
        let java: BTreeMap<String, JavaResult> = match std::fs::read_to_string(fixture_file) {
            Ok(t) => fixture::read_bundle(&t)
                .unwrap_or_else(|e| die(&format!("{}: {e}", fixture_file.display()))),
            Err(_) => {
                eprintln!(
                    "qvto: no fixture {} (run `qvto conform record`); no differential comparison",
                    fixture_file.display()
                );
                BTreeMap::new()
            }
        };
        let mut engine: Box<dyn Engine> = match o.engine.as_deref().unwrap_or("qvto") {
            "stub" => Box::new(StubEngine),
            "qvto" => Box::new(engine::QvtoEngine::new(root, qvto_runtime::Mode::Compat)),
            "qvto-fast" => Box::new(engine::QvtoEngine::new(root, qvto_runtime::Mode::Fast)),
            e if e.starts_with("replay:") => {
                let d = PathBuf::from(&e["replay:".len()..]).join(name);
                Box::new(
                    JavaReplay::new(&d)
                        .unwrap_or_else(|err| die(&format!("{}: {err}", d.display()))),
                )
            }
            e => die(&format!("unknown engine {e}")),
        };
        engine_name = engine.name().to_string();
        let select = |c: &Case| {
            (o.only.is_empty() || o.only.contains(&c.id))
                && (o.prefixes.is_empty() || o.prefixes.iter().any(|p| c.id.starts_with(p)))
        };
        all.extend(report::run_cases(
            engine.as_mut(),
            &cases,
            root,
            &models,
            &java,
            &select,
        ));
    }
    let md = report::markdown(&engine_name, &all, o.verbose);
    match &o.report {
        Some(p) => {
            std::fs::write(p, &md).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
            let first: Vec<&str> = md.lines().take(3).collect();
            println!("{}", first.join("\n"));
        }
        None => print!("{md}"),
    }
    if let Some(p) = &o.tsv {
        std::fs::write(p, report::tsv(&all))
            .unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    }
}

/// `qvto bench [--suite eclipse|corpus] --only ID,... [--repeat N] [--java]`: runs cases N
/// times with the engine (and with `--java` in one JVM of the reference runner, the case
/// repeated N times so that the later runs are warm) and prints the run times.
fn bench(args: &[String]) {
    let mut suite = "eclipse".to_string();
    let mut only: Vec<String> = Vec::new();
    let mut repeat = 20usize;
    let mut java = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--suite" => {
                i += 1;
                suite = args[i].clone();
            }
            "--only" => {
                i += 1;
                only.extend(args[i].split(',').map(String::from));
            }
            "--repeat" => {
                i += 1;
                repeat = args[i].parse().unwrap_or(20);
            }
            "--java" => java = true,
            a => die(&format!("unknown argument {a}")),
        }
        i += 1;
    }
    let root =
        find_root(Path::new(".")).unwrap_or_else(|| die("not inside the qvto-rs repository"));
    let s = SUITES
        .iter()
        .find(|s| s.name == suite)
        .unwrap_or_else(|| die("unknown suite"));
    let cases: Vec<Case> = read_cases(&root.join(s.cases)).unwrap_or_else(|e| die(&e.to_string()));
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), &root)
        .unwrap_or_else(|e| die(&format!("{}: {e}", paths::SUITE_CONF)));
    let median = |v: &mut Vec<f64>| -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() {
            f64::NAN
        } else {
            v[v.len() / 2]
        }
    };
    let mut engine = engine::QvtoEngine::new(&root, qvto_runtime::Mode::Compat);
    let mut dup = String::new();
    for c in cases.iter().filter(|c| only.contains(&c.id)) {
        let conf_models;
        let models = match &c.conf {
            Some(cf) => {
                conf_models = qvto_emf::config::Config::read(&root.join(cf), &root)
                    .unwrap_or_else(|e| die(&format!("{cf}: {e}")));
                &conf_models
            }
            None => &models,
        };
        let env = qvto_testkit::engine::CaseEnv {
            case: c,
            base: &root,
            models,
        };
        let mut runs = Vec::new();
        let mut compiles = Vec::new();
        for _ in 0..repeat {
            let o = engine.run(&env);
            if let Some(t) = o.run_time {
                runs.push(t.as_secs_f64() * 1e3);
            }
            if let Some(t) = o.compile_time {
                compiles.push(t.as_secs_f64() * 1e3);
            }
        }
        let first_run = runs.first().copied().unwrap_or(f64::NAN);
        println!(
            "qvto  {:40} compile {:8.3} ms  run first {:8.3} ms  median {:8.3} ms",
            c.id,
            median(&mut compiles),
            first_run,
            median(&mut runs)
        );
        for k in 0..repeat {
            let mut c2 = c.clone();
            c2.id = format!("{}~{k}", c.id);
            qvto_testkit::case::write_case(&c2, &mut dup);
        }
    }
    if !java {
        return;
    }
    let dir = root.join("target/qvto-bench");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("bench.cases");
    std::fs::write(&file, &dup).unwrap_or_else(|e| die(&e.to_string()));
    let out = dir.join("java");
    let _ = std::fs::remove_dir_all(&out);
    let run = JavaRun {
        base: root.clone(),
        cases: file,
        out: out.clone(),
        only: Vec::new(),
        prefixes: Vec::new(),
        tape: false,
        save_xmi: false,
        jvm_opts: None,
    };
    if let Err(e) = run.run() {
        die(&e);
    }
    let results = fixture::read_result_dir(&out).unwrap_or_else(|e| die(&e.to_string()));
    for id in &only {
        let mut runs: Vec<(usize, f64)> = results
            .iter()
            .filter_map(|(k, r)| {
                let (base, n) = k.split_once('~')?;
                if base != id {
                    return None;
                }
                Some((n.parse().ok()?, r.get("run-ms")?.parse().ok()?))
            })
            .collect();
        runs.sort_by_key(|x| x.0);
        let first = runs.first().map(|x| x.1).unwrap_or(f64::NAN);
        let mut warm: Vec<f64> = runs.iter().skip(runs.len() / 2).map(|x| x.1).collect();
        println!(
            "java  {:40} run first {:8.3} ms  median of the second half {:8.3} ms",
            id,
            first,
            median(&mut warm)
        );
    }
}

/// `qvto run FILE.qvto [MODEL...] [--repeat N] [--fast]`: compiles and runs a transformation
/// (models bound positionally to the in/inout parameters; metamodels by namespace URI from
/// the suite configuration); prints the log, the outcome and the run times.
fn run_cmd(args: &[String]) {
    let mut files = Vec::new();
    let mut repeat = 1usize;
    let mut mode = qvto_runtime::Mode::Compat;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repeat" => {
                i += 1;
                repeat = args.get(i).and_then(|r| r.parse().ok()).unwrap_or(1);
            }
            "--fast" => mode = qvto_runtime::Mode::Fast,
            a => files.push(a.to_string()),
        }
        i += 1;
    }
    let Some(file) = files.first().cloned() else {
        die("usage: qvto run FILE.qvto [MODEL...] [--repeat N] [--fast]")
    };
    let root =
        find_root(Path::new(".")).unwrap_or_else(|| die("not inside the qvto-rs repository"));
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), &root)
        .unwrap_or_else(|e| die(&format!("{}: {e}", paths::SUITE_CONF)));
    let path = PathBuf::from(&file);
    let resolver = qvto::units::FsResolver {
        bundles: qvto::units::FsResolver::bundles_below(&root.join("reference/eclipse-qvto/tests")),
        from_folder: false,
    };
    let main = resolver
        .unit_source(&path)
        .unwrap_or_else(|e| die(&format!("{file}: {e}")));
    let uris = qvto_sema::scan_metamodel_uris(&main, &resolver);
    let fresh = || {
        let mut rs = models.resource_set_for(&path);
        for uri in &uris {
            let _ = engine::load_metamodel(&mut rs, uri);
        }
        rs
    };
    let rs = fresh();
    let t0 = std::time::Instant::now();
    let c = qvto::compile(main, &resolver, rs.graph.meta().clone());
    let tc = t0.elapsed();
    for u in &c.units {
        for p in &u.problems {
            eprintln!(
                "{} {}:{}: {}",
                if p.is_error() { "error" } else { "warning" },
                u.source.file_name,
                p.line(&u.lines),
                p.message
            );
        }
    }
    let Some(p) = c.program else {
        die(&format!(
            "no program {}",
            c.bytecode_error.unwrap_or_default()
        ))
    };
    eprintln!("compile: {:.3} ms", tc.as_secs_f64() * 1e3);
    for run in 0..repeat {
        let mut rs = fresh();
        let main_mod = &p.ir.modules[p.ir.main_module.index()];
        let mut inputs = files[1..].iter();
        let mut a = Vec::new();
        for mp in &main_mod.model_params {
            if mp.direction == qvto_sema::ir::Direction::Out {
                a.push(Vec::new());
                continue;
            }
            match inputs.next() {
                Some(f) => {
                    let r = rs
                        .load_file(Path::new(f))
                        .unwrap_or_else(|e| die(&format!("{f}: {e}")));
                    a.push(rs.graph.roots(r).to_vec());
                }
                None => a.push(Vec::new()),
            }
        }
        rs.resolve_all();
        let opts = qvto::RunOptions {
            mode,
            ..qvto::RunOptions::default()
        };
        let t1 = std::time::Instant::now();
        let mut vm = qvto::Vm::new(&p, &mut rs.graph, opts);
        let out = vm.run_transformation(a);
        let dt = t1.elapsed();
        if run + 1 == repeat {
            print!("{}", vm.log);
            for e in &vm.errors {
                eprintln!("evaluation: {e}");
            }
            println!(
                "severity {} code {} message {} traces {}",
                out.severity, out.code, out.message, out.trace_records
            );
        }
        eprintln!("run {run}: {:.3} ms", dt.as_secs_f64() * 1e3);
    }
}

/// `qvto disasm FILE.qvto [nsURI-or-.ecore ...]`: compiles a transformation (imports relative
/// to its folder, metamodels from the suite configuration) and prints the program.
fn disasm(args: &[String]) {
    let Some(file) = args.first() else {
        die("usage: qvto disasm FILE.qvto")
    };
    let root =
        find_root(Path::new(".")).unwrap_or_else(|| die("not inside the qvto-rs repository"));
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), &root)
        .unwrap_or_else(|e| die(&format!("{}: {e}", paths::SUITE_CONF)));
    let path = PathBuf::from(file);
    let mut rs = models.resource_set_for(&path);
    let resolver = qvto::units::FsResolver {
        bundles: qvto::units::FsResolver::bundles_below(&root.join("reference/eclipse-qvto/tests")),
        from_folder: false,
    };
    let main = resolver
        .unit_source(&path)
        .unwrap_or_else(|e| die(&format!("{file}: {e}")));
    for uri in qvto_sema::scan_metamodel_uris(&main, &resolver)
        .iter()
        .chain(args[1..].iter())
    {
        if let Err(e) = engine::load_metamodel(&mut rs, uri) {
            eprintln!("metamodel {uri}: {e}");
        }
    }
    let c = qvto::compile(main, &resolver, rs.graph.meta().clone());
    for u in &c.units {
        for p in &u.problems {
            println!(
                "{} {}:{}: {}",
                if p.is_error() { "error" } else { "warning" },
                u.source.file_name,
                p.line(&u.lines),
                p.message
            );
        }
    }
    let Some(p) = c.program else {
        die(&format!(
            "no program {}",
            c.bytecode_error.unwrap_or_default()
        ))
    };
    for (i, src) in p.ir.sources.iter().enumerate() {
        println!("source {i} {} ({})", src.name, src.qualified_name);
    }
    for (i, o) in p.ir.operations.iter().enumerate() {
        println!(
            "op {i} {} module {} source {} context {:?} vtable {:?} overrides {:?}",
            o.name, o.module.0, o.source.0, o.context, o.vtable, o.overrides
        );
    }
    for (i, m) in p.ir.modules.iter().enumerate() {
        println!(
            "module {i} {} source {} {:?} params {:?} entry {:?}",
            m.name,
            m.source.0,
            m.kind,
            m.model_params
                .iter()
                .map(|mp| format!("{:?} {}", mp.direction, mp.name))
                .collect::<Vec<_>>(),
            m.entry
        );
    }
    print!("{}", qvto_bytecode::disasm::program(&p));
}
