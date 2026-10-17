//! `qvto`: the command line: `qvto conform` (the conformance scoreboard and the Java reference
//! runner's fixtures), `qvto run` (compile and run a transformation), `qvto disasm` (the
//! compiled program), `qvto bench` (run times of cases against the warm Java runner).

mod bbox;
mod bench;
mod engine;
mod fuzz;
mod robust;

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
  qvto bench [gen] [--file F | --suite eclipse|corpus] [--only ID,...] [--group G,...]
             [--modes compat,fast] [--java] [--java-cold] [--no-rust] [--compiles K]
             [--runs N] [--budget-ms B] [--report FILE.md]
      The benchmark suite (qvto-cli/bench/workloads.bench): compile and run times in
      compat and fast mode, with --java against the warm reference runner (one JVM), with
      --java-cold a fresh JVM per workload. Run it pinned: taskset -c 2,3 qvto bench --java.
  qvto conform [--suite eclipse|corpus|fuzz|all] [--cases FILE] [--fixture FILE]
               [--engine qvto|qvto-fast|stub|replay:DIR] [--only ID,...] [--prefix P]...
               [--report FILE.md] [--tsv FILE] [--verbose]
      Runs the engine on the cases and prints the scoreboard: per case the Eclipse
      driver's checks and the strict comparison with the Java runner's fixture.
  qvto conform record [--suite eclipse|corpus|fuzz|all] [--only ID,...] [--prefix P]... [--runs N]
      Runs the Java reference runner N times (default 5, one JVM per run), marks
      outputs that differ between the runs as unstable and writes the fixture
      bundle; the full results (tapes, XMI) stay in target/qvto-testkit/java/<suite>.
  qvto conform java [--suite ...] [--out DIR] [--only ID,...] [--prefix P]... [--save-xmi]
      Runs the Java reference runner once into DIR.
  qvto fuzz expr|prog [--seed N] [--minutes M | --batches B] [--cases C] [--stmts S] [--depth D] [--out DIR] [--keep]
      Differential fuzzing against the Java runner: random expressions or programs over the fuzz
      metamodel; findings (minimised) in DIR/findings.tsv and DIR/findings/.
  qvto fuzz recheck DIR...
      Runs saved findings of a campaign again.
  qvto fuzz corpus --cases FILE [--out DIR] [--only ID,...] [--reuse] [--tape]
      Compares compile problems (and runs) of a case file with the Java runner (report.tsv).
  qvto fuzz robust [--workers N] [--minutes M] [--seed N] [--out DIR] [--mem-mb MB]
      Robustness fuzzing of parser, analyser, compiler and VM (no panics, hangs or aborts).
  qvto fuzz robust-replay FILE.qvto...
      Runs the robustness pipeline on files.";

struct Suite {
    name: &'static str,
    cases: &'static str,
    fixture: &'static str,
}

const SUITES: [Suite; 3] = [
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
    Suite {
        name: "fuzz",
        cases: paths::FUZZ_CASES,
        fixture: paths::FUZZ_FIXTURE,
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
        Some("bench") => bench::bench(&args[1..]),
        Some("fuzz") => fuzz::main(&args[1..]),
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
                let cache = root.join(paths::JAVA_CACHE).join(name);
                let partial = !o.only.is_empty() || !o.prefixes.is_empty();
                let run = JavaRun {
                    base: root.clone(),
                    cases: cases.clone(),
                    // a partial recording goes to a side directory merged into the cache
                    out: if partial {
                        cache.with_extension("partial")
                    } else {
                        cache.clone()
                    },
                    only: o.only.clone(),
                    prefixes: o.prefixes.clone(),
                    tape: true,
                    save_xmi: true,
                    // the fuzz regressions run with constant identity hashes on both sides
                    // (deterministic: no reruns needed to find unstable outputs)
                    jvm_opts: (name == "fuzz")
                        .then(|| "-XX:+UnlockExperimentalVMOptions -XX:hashCode=2".to_string()),
                    timeout: None,
                };
                let _ = std::fs::remove_dir_all(&run.out);
                let runs = if name == "fuzz" { 1 } else { o.runs };
                let r = record(&run, fixture, runs, partial);
                if partial && r.is_ok() {
                    if let Ok(d) = std::fs::read_dir(&run.out) {
                        for e in d.flatten() {
                            if e.path().is_dir() {
                                let target = cache.join(e.file_name());
                                let _ = std::fs::remove_dir_all(&target);
                                let _ = std::fs::rename(e.path(), &target);
                            }
                        }
                    }
                    let _ = std::fs::remove_dir_all(&run.out);
                }
                match r {
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
                    timeout: None,
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
            e @ ("qvto" | "qvto-fast") => {
                let mode = if e == "qvto" { qvto_runtime::Mode::Compat } else { qvto_runtime::Mode::Fast };
                let mut q = engine::QvtoEngine::new(root, mode);
                if name == "fuzz" {
                    q.constant_identity = Some(1);
                }
                Box::new(q)
            }
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
    let catalog = engine::cli_catalog(&root);
    let resolver = engine::cli_resolver(&resolver, &catalog);
    let uris = qvto_sema::scan_metamodel_uris(&main, &resolver);
    let fresh = || {
        let mut rs = models.resource_set_for(&path);
        for uri in &uris {
            let _ = engine::load_metamodel(&mut rs, uri);
        }
        rs.ecore_resource();
        rs
    };
    let rs = fresh();
    let t0 = std::time::Instant::now();
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let c = qvto::compile_with_metamodel(main, &resolver, rs.graph.meta().clone(), &ops, &qvto_emf::operations::meta_derivations(&rs.graph));
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
                    let r = engine::get_or_load(&mut rs, Path::new(f))
                        .unwrap_or_else(|e| die(&format!("{f}: {e}")));
                    a.push(rs.graph.roots(r).to_vec());
                }
                None => a.push(Vec::new()),
            }
        }
        rs.resolve_all();
        let opts = qvto::RunOptions {
            mode,
            // the main thread's stack (8 MB): Eclipse's depth of 1 000 fits
            max_native_stack: Some(6 << 20),
            ..qvto::RunOptions::default()
        };
        let t1 = std::time::Instant::now();
        let mut vm = qvto::Vm::new(&p, &mut rs.graph, opts);
        vm.set_natives(std::rc::Rc::new(bbox::natives()));
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
    let catalog = engine::cli_catalog(&root);
    let resolver = engine::cli_resolver(&resolver, &catalog);
    for uri in qvto_sema::scan_metamodel_uris(&main, &resolver)
        .iter()
        .chain(args[1..].iter())
    {
        if let Err(e) = engine::load_metamodel(&mut rs, uri) {
            eprintln!("metamodel {uri}: {e}");
        }
    }
    rs.ecore_resource();
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let c = qvto::compile_with_metamodel(main, &resolver, rs.graph.meta().clone(), &ops, &qvto_emf::operations::meta_derivations(&rs.graph));
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
