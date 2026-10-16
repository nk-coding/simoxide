//! `qvto`: the command line. Implemented so far: `qvto conform` (the conformance scoreboard and
//! the Java reference runner's fixtures). `run`, `check`, `bench`, `disasm` come with the engine.

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
  qvto conform [--suite eclipse|corpus|all] [--cases FILE] [--fixture FILE]
               [--engine stub|replay:DIR] [--only ID,...] [--prefix P]...
               [--report FILE.md] [--tsv FILE] [--verbose]
      Runs the engine on the cases and prints the scoreboard: per case the Eclipse
      driver's checks and the strict comparison with the Java runner's fixture.
  qvto conform record [--suite eclipse|corpus|all] [--only ID,...] [--prefix P]... [--runs N]
      Runs the Java reference runner N times (default 3, one JVM per run), marks
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
    Suite { name: "eclipse", cases: paths::ECLIPSE_CASES, fixture: paths::ECLIPSE_FIXTURE },
    Suite { name: "corpus", cases: paths::CORPUS_CASES, fixture: paths::CORPUS_FIXTURE },
];

fn die(msg: &str) -> ! {
    eprintln!("qvto: {msg}");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("conform") => conform(&args[1..]),
        Some("-h" | "--help") | None => {
            println!("{USAGE}");
        }
        Some(c) => die(&format!("unknown command {c} (run, check, bench come with the engine)\n{USAGE}")),
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
    let mut o = Opts { runs: 3, ..Opts::default() };
    let mut i = 0;
    let val = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i).cloned().unwrap_or_else(|| die(&format!("{} needs a value", args[*i - 1])))
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
            "--runs" => o.runs = val(&mut i).parse().unwrap_or_else(|_| die("--runs needs a number")),
            a => die(&format!("unknown argument {a}\n{USAGE}")),
        }
        i += 1;
    }
    o
}

/// The (name, case file, fixture) triples selected by the options.
fn suites(o: &Opts, root: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    if let Some(c) = &o.cases {
        let f = o.fixture.clone().unwrap_or_else(|| c.with_extension("fixture.txt"));
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
    let root = find_root(Path::new(".")).unwrap_or_else(|| die("not inside the qvto-rs repository"));
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
                    Ok(log) => print!("{}", log.lines().filter(|l| !l.starts_with("pass\t")).collect::<Vec<_>>().join("\n") + "\n"),
                    Err(e) => die(&e),
                }
            }
        }
        Some("java") => {
            for (name, cases, _) in &suites {
                let run = JavaRun {
                    base: root.clone(),
                    cases: cases.clone(),
                    out: o.out.clone().map(|d| d.join(name)).unwrap_or_else(|| root.join(paths::JAVA_CACHE).join(name)),
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
    let pcm_conf = root.join("target/pcm/pcm.conf");
    let mut all = Vec::new();
    let mut engine_name = String::new();
    for (name, cases_file, fixture_file) in suites {
        let cases: Vec<Case> = read_cases(cases_file).unwrap_or_else(|e| die(&format!("{}: {e}", cases_file.display())));
        let java: BTreeMap<String, JavaResult> = match std::fs::read_to_string(fixture_file) {
            Ok(t) => fixture::read_bundle(&t).unwrap_or_else(|e| die(&format!("{}: {e}", fixture_file.display()))),
            Err(_) => {
                eprintln!("qvto: no fixture {} (run `qvto conform record`); no differential comparison", fixture_file.display());
                BTreeMap::new()
            }
        };
        let mut engine: Box<dyn Engine> = match o.engine.as_deref().unwrap_or("stub") {
            "stub" => Box::new(StubEngine),
            e if e.starts_with("replay:") => {
                let d = PathBuf::from(&e["replay:".len()..]).join(name);
                Box::new(JavaReplay::new(&d).unwrap_or_else(|err| die(&format!("{}: {err}", d.display()))))
            }
            e => die(&format!("unknown engine {e}")),
        };
        engine_name = engine.name().to_string();
        // corpus cases resolve PCM metamodels through the PCM configuration (crates/qvto-emf/tools/pcm-setup.sh)
        let suite_models = if name == "corpus" && pcm_conf.is_file() {
            qvto_emf::config::Config::read(&pcm_conf, root).unwrap_or_else(|e| die(&format!("{}: {e}", pcm_conf.display())))
        } else {
            models.clone()
        };
        let select = |c: &Case| {
            (o.only.is_empty() || o.only.contains(&c.id)) && (o.prefixes.is_empty() || o.prefixes.iter().any(|p| c.id.starts_with(p)))
        };
        all.extend(report::run_cases(engine.as_mut(), &cases, root, &suite_models, &java, &select));
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
        std::fs::write(p, report::tsv(&all)).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    }
}
