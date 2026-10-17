//! `qvto fuzz`: differential fuzzing campaigns against the Java reference runner (PLAN Q7).
//!
//! ```text
//! qvto fuzz expr [--seed N] [--minutes M | --batches B] [--cases C] [--stmts S] [--depth D] [--out DIR]
//! qvto fuzz prog [--seed N] [--minutes M | --batches B] [--cases C] [--out DIR]
//! ```
//!
//! A campaign generates batches of cases (`qvto_testkit::fuzz`), runs the Java runner once per
//! batch (one JVM, identity-hash tapes on), runs the engine in compat mode on each case with the
//! Java run's tape, and compares strictly (status, log, output dump, compile problems with their
//! messages). Expression campaigns compare the log per statement label and minimise every
//! differing statement (`E::reductions`, one candidate per case, the Java runner on all
//! candidates at once, the smallest still-differing candidate wins, until nothing smaller
//! differs). Findings go to `<out>/findings.tsv` and `<out>/findings/` (a self-contained case per
//! finding, minimised where possible); batch directories without findings are deleted.

use crate::engine::QvtoEngine;
use qvto_runtime::Mode;
use qvto_testkit::case::{Case, read_cases};
use qvto_testkit::compare::{Aspect, Differential, differential};
use qvto_testkit::engine::{CaseEnv, Engine, Outcome, Status};
use qvto_testkit::fixture::{self, JavaResult};
use qvto_testkit::fuzz::expr::{self, Gen, Stmt};
use qvto_testkit::fuzz::{self as fz, Rng};
use qvto_testkit::java::JavaRun;
use qvto_testkit::paths;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Identity hashes: by default the Java side runs with `-XX:hashCode=2` (every identity hash
/// is 1) and the engine with a constant source, which makes identity-hash orders reproducible;
/// `--tape` runs with HotSpot's default hashes and replays the Java run's tape instead.
static TAPE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Opts {
    pub kind: String,
    pub seed: u64,
    pub minutes: Option<f64>,
    pub batches: Option<usize>,
    pub cases: usize,
    pub stmts: usize,
    pub depth: u32,
    pub out: PathBuf,
    pub keep: bool,
    pub minimize: bool,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        kind: args.first().cloned().unwrap_or_default(),
        seed: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1),
        minutes: None,
        batches: None,
        cases: 40,
        stmts: 40,
        depth: 4,
        out: PathBuf::new(),
        keep: false,
        minimize: true,
    };
    let mut i = 1;
    let val = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i)
            .cloned()
            .unwrap_or_else(|| crate::die("missing value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => o.seed = val(&mut i).parse().unwrap_or_else(|_| crate::die("--seed")),
            "--minutes" => o.minutes = val(&mut i).parse().ok(),
            "--batches" => o.batches = val(&mut i).parse().ok(),
            "--cases" => o.cases = val(&mut i).parse().unwrap_or(40),
            "--stmts" => o.stmts = val(&mut i).parse().unwrap_or(40),
            "--depth" => o.depth = val(&mut i).parse().unwrap_or(4),
            "--out" => o.out = val(&mut i).into(),
            "--keep" => o.keep = true,
            "--no-minimize" => o.minimize = false,
            "--tape" => TAPE_MODE.store(true, std::sync::atomic::Ordering::Relaxed),
            a => crate::die(&format!("fuzz: unknown argument {a}")),
        }
        i += 1;
    }
    o
}

pub fn main(args: &[String]) {
    let root = qvto_testkit::find_root(Path::new("."))
        .unwrap_or_else(|| crate::die("not inside the qvto-rs repository"));
    match args.first().map(String::as_str) {
        Some("robust") => return crate::robust::main(&root, &args[1..]),
        Some("robust-worker") => return crate::robust::worker(&root, &args[1..]),
        Some("robust-replay") => return crate::robust::replay(&root, &args[1..]),
        Some("recheck") => return recheck(&root, &args[1..]),
        Some("corpus") => return corpus(&root, &args[1..]),
        Some("expr" | "prog") => {}
        k => crate::die(&format!(
            "fuzz: unknown campaign {k:?} (expr, prog, robust, robust-replay)"
        )),
    }
    let mut o = parse(args);
    if o.out.as_os_str().is_empty() {
        o.out = root.join("target/qvto-fuzz").join(&o.kind);
    }
    std::fs::create_dir_all(o.out.join("findings")).unwrap_or_else(|e| crate::die(&e.to_string()));
    campaign(&root, &o);
}

/// `qvto fuzz recheck DIR...`: runs both sides on the cases of finding directories (every
/// `X.qvto` with its `X.xmi`) and prints what differs.
fn recheck(root: &Path, dirs: &[String]) {
    for d in dirs {
        let dir = std::path::absolute(d).unwrap_or_else(|e| crate::die(&e.to_string()));
        let mut cases = Vec::new();
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| crate::die(&format!("{d}: {e}")))
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "qvto"))
            .collect();
        names.sort();
        for p in names {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let xmi = format!("{stem}.xmi");
            cases.push(GenCase {
                id: stem.clone(),
                files: Vec::new(),
                qvto: format!("{stem}.qvto"),
                input: dir.join(&xmi).is_file().then_some(xmi),
                stmts: Vec::new(),
            });
        }
        let cf = write_batch(root, &dir, &cases);
        match run_batch(root, &dir, &cf) {
            Ok(ran) => {
                for (id, r) in &ran {
                    let j = r.java.as_ref();
                    let same = r.diff.as_ref().is_some_and(|d| d.equal()) && r.sema.is_none();
                    println!("== {d} {id}: {}", if same { "equal" } else { "DIFFERS" });
                    if !same {
                        if let Some(s) = &r.sema {
                            println!("sema: {s}");
                        }
                        if let Some(df) = &r.diff {
                            println!("{}", df.first_difference().unwrap_or_default());
                        }
                        println!(
                            "--- java log\n{}--- engine log\n{}",
                            j.map(|j| j.log()).unwrap_or(""),
                            r.outcome.log
                        );
                    }
                }
            }
            Err(e) => println!("== {d}: {e}"),
        }
    }
}

/// `qvto fuzz corpus [--cases FILE] [--out DIR] [--only ID,..] [--reuse]`: both sides on a
/// case file of existing transformations (default `cases/external.cases`); compile problems
/// compared exactly (severity, unit, line, offset, length, message), run cases also by
/// outcome, log and output models. Writes `DIR/report.tsv` (one row per case) and prints a
/// summary. `--reuse` keeps the Java results of a previous run in `DIR/java`. Identity hashes
/// are constant on both sides (`--tape`: HotSpot's, replayed from the Java run's tape).
fn corpus(root: &Path, args: &[String]) {
    let mut cases = root.join("qvto-testkit/cases/external.cases");
    let mut out = root.join("target/qvto-fuzz/corpus");
    let mut only: Vec<String> = Vec::new();
    let mut reuse = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cases" => {
                i += 1;
                cases = root.join(&args[i]);
            }
            "--out" => {
                i += 1;
                out = root.join(&args[i]);
            }
            "--only" => {
                i += 1;
                only.extend(args[i].split(',').map(String::from));
            }
            "--reuse" => reuse = true,
            "--tape" => TAPE_MODE.store(true, std::sync::atomic::Ordering::Relaxed),
            a => crate::die(&format!("fuzz corpus: unknown argument {a}")),
        }
        i += 1;
    }
    let _ = std::fs::create_dir_all(&out);
    let cf = if only.is_empty() {
        cases.clone()
    } else {
        let all = read_cases(&cases).unwrap_or_else(|e| crate::die(&e.to_string()));
        let mut t = String::new();
        for c in all.iter().filter(|c| only.contains(&c.id)) {
            qvto_testkit::case::write_case(c, &mut t);
        }
        let p = out.join("selected.cases");
        let _ = std::fs::write(&p, t);
        p
    };
    // constant identity hashes on both sides (`-XX:hashCode=2`), as the fuzz campaigns: object
    // hashes in logs and identity-hash orders are reproducible (the tape covers only the
    // objects of the inputs, the outputs and their metamodels)
    let ran = match run_batch_opts(root, &out, &cf, reuse, false) {
        Ok(r) => r,
        Err(e) => crate::die(&e),
    };
    let mut rep = String::from("id\tverdict\tjava_errors\tengine_errors\tdetail\n");
    // every problem only one side reports: id, side, unit, line, offset, length, severity, message
    let mut mis = String::new();
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for (id, r) in &ran {
        let je = r
            .java
            .as_ref()
            .map(|j| j.problems().iter().filter(|p| p.error).count());
        let ee = r.outcome.problems.iter().filter(|p| p.error).count();
        let jv = r
            .java
            .as_ref()
            .map(|j| j.verdict().to_string())
            .unwrap_or_default();
        let (verdict, detail) = if r.java.is_none() || jv == "error" || jv == "timeout" {
            (
                "java-error",
                r.java
                    .as_ref()
                    .and_then(|j| j.get("detail"))
                    .unwrap_or("")
                    .to_string(),
            )
        } else if let Status::Crashed(m) = &r.outcome.status {
            ("crash", m.clone())
        } else if let Some(s) = &r.sema {
            if (je.unwrap_or(0) > 0) != (ee > 0) {
                ("verdict-differs", s.clone())
            } else {
                ("problems-differ", s.clone())
            }
        } else if r.diff.as_ref().is_some_and(|d| !d.equal())
            && !r
                .java
                .as_ref()
                .is_some_and(|j| j.problems().iter().any(|p| p.error))
        {
            (
                "run-differs",
                r.diff
                    .as_ref()
                    .unwrap()
                    .first_difference()
                    .unwrap_or_default(),
            )
        } else {
            ("equal", String::new())
        };
        *tally.entry(verdict).or_default() += 1;
        if r.sema.is_some()
            && let Some(j) = &r.java
        {
            let jp = j.problems();
            for (side, mine, other) in [
                ("engine", &r.outcome.problems, &jp),
                ("java", &jp, &r.outcome.problems),
            ] {
                for p in mine.iter().filter(|p| !other.contains(p)) {
                    let _ = writeln!(
                        mis,
                        "{id}\t{side}\t{}\t{}\t{}\t{}\t{}\t{}",
                        p.unit,
                        p.line,
                        p.offset,
                        p.length,
                        if p.error { "E" } else { "W" },
                        one_line(&p.message)
                    );
                }
            }
        }
        let _ = writeln!(
            rep,
            "{id}\t{verdict}\t{}\t{ee}\t{}",
            je.map(|x| x.to_string()).unwrap_or_default(),
            one_line(&detail)
        );
    }
    let _ = std::fs::write(out.join("report.tsv"), rep);
    let _ = std::fs::write(out.join("mismatches.tsv"), mis);
    println!(
        "{} cases: {tally:?}; report in {}",
        ran.len(),
        out.join("report.tsv").display()
    );
}

/// A generated case before it is written.
pub struct GenCase {
    pub id: String,
    /// File name and contents, in the batch directory.
    pub files: Vec<(String, String)>,
    pub qvto: String,
    pub input: Option<String>,
    /// The statements of an expression case (labels `@e<k>@`).
    pub stmts: Vec<Stmt>,
}

/// The result of one case on both sides.
pub struct Ran {
    pub java: Option<JavaResult>,
    pub outcome: Outcome,
    pub diff: Option<Differential>,
    /// Compile problems with messages, if they differ.
    pub sema: Option<String>,
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

/// Writes the cases of a batch into `dir` and returns the case file.
pub fn write_batch(root: &Path, dir: &Path, cases: &[GenCase]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| crate::die(&e.to_string()));
    let _ = std::fs::write(dir.join("fuzz.ecore"), fz::FUZZ_ECORE);
    let mut cf = String::new();
    for c in cases {
        for (name, text) in &c.files {
            let _ = std::fs::write(dir.join(name), text);
        }
        let _ = writeln!(
            cf,
            "case {}\ndriver fuzz\nkind fuzz\ndir {}\nqvto {}",
            c.id,
            rel(root, dir),
            c.qvto
        );
        if let Some(i) = &c.input {
            let _ = writeln!(cf, "in {i}");
        }
        let _ = writeln!(cf, "mmfile fuzz.ecore\nend\n");
    }
    let p = dir.join("cases.txt");
    let _ = std::fs::write(&p, cf);
    p
}

/// Runs the Java runner and the engine (compat mode) on a case file.
pub fn run_batch(
    root: &Path,
    dir: &Path,
    case_file: &Path,
) -> Result<BTreeMap<String, Ran>, String> {
    run_batch_opts(root, dir, case_file, false, false)
}

/// [`run_batch`]; `reuse`: keep the Java results already in `dir/java`; `default_hashes`:
/// HotSpot's identity hashes with the tape (not `-XX:hashCode=2`).
pub fn run_batch_opts(
    root: &Path,
    dir: &Path,
    case_file: &Path,
    reuse: bool,
    default_hashes: bool,
) -> Result<BTreeMap<String, Ran>, String> {
    let jdir = dir.join("java");
    let tape = tape_mode() || default_hashes;
    if !reuse {
        let _ = std::fs::remove_dir_all(&jdir);
        let run = JavaRun {
            base: root.to_path_buf(),
            cases: case_file.to_path_buf(),
            out: jdir.clone(),
            tape: true,
            timeout: Some(30),
            jvm_opts: (!tape)
                .then(|| "-XX:+UnlockExperimentalVMOptions -XX:hashCode=2".to_string()),
            ..JavaRun::default()
        };
        run.run()?;
    }
    let java = fixture::read_result_dir(&jdir).map_err(|e| e.to_string())?;
    let cases: Vec<Case> = read_cases(case_file).map_err(|e| e.to_string())?;
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), root)
        .map_err(|e| e.to_string())?;
    if let Some(why) = Mode::Compat.unavailable_reason() {
        return Err(format!(
            "differential fuzzing compares compat mode with Java: {why}"
        ));
    }
    let mut engine = QvtoEngine::new(root, Mode::Compat);
    engine.tape_dirs.push(jdir.clone());
    if !tape {
        engine.constant_identity = Some(1);
    }
    let mut out = BTreeMap::new();
    let mut confs: BTreeMap<String, qvto_emf::config::Config> = BTreeMap::new();
    for case in &cases {
        if let Some(c) = &case.conf
            && !confs.contains_key(c)
        {
            let m = qvto_emf::config::Config::read(&root.join(c), root)
                .map_err(|e| format!("{c}: {e}"))?
                .with_base(&models);
            confs.insert(c.clone(), m);
        }
        let env = CaseEnv {
            case,
            base: root,
            models: case
                .conf
                .as_ref()
                .and_then(|c| confs.get(c))
                .unwrap_or(&models),
        };
        let outcome =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.run(&env))) {
                Ok(o) => o,
                Err(p) => {
                    let msg = p
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    Outcome::with_status(Status::Crashed(format!("panic: {msg}")))
                }
            };
        let j = java.get(&case.id).cloned();
        let diff = j.as_ref().map(|j| differential(case, &outcome, j));
        let sema = j.as_ref().and_then(|j| sema_difference(&outcome, j));
        out.insert(
            case.id.clone(),
            Ran {
                java: j,
                outcome,
                diff,
                sema,
            },
        );
    }
    Ok(out)
}

fn tape_mode() -> bool {
    TAPE_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Compile problems compared with their messages (the differential compares severity and line).
fn sema_difference(o: &Outcome, j: &JavaResult) -> Option<String> {
    let key = |p: &qvto_testkit::fixture::Problem| {
        (
            p.unit.clone(),
            p.line,
            p.offset,
            p.length,
            p.error,
            p.message.clone(),
        )
    };
    let mut a: Vec<_> = o.problems.iter().map(key).collect();
    let mut b: Vec<_> = j.problems().iter().map(key).collect();
    a.sort();
    b.sort();
    if a == b {
        return None;
    }
    let only_a: Vec<_> = a.iter().filter(|x| !b.contains(x)).collect();
    let only_b: Vec<_> = b.iter().filter(|x| !a.contains(x)).collect();
    Some(format!("engine only {only_a:?}; java only {only_b:?}"))
}

fn has_errors(r: &Ran) -> bool {
    r.outcome.problems.iter().any(|p| p.error)
        || r.java
            .as_ref()
            .is_some_and(|j| j.problems().iter().any(|p| p.error))
}

/// The log split by statement labels (`@e12@`): every line that contains a label starts that
/// label's segment; other lines continue the current one.
pub fn segments(log: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut cur = String::from("-");
    for line in log.split_inclusive('\n') {
        if let Some(l) = find_label(line) {
            cur = l;
        }
        out.entry(cur.clone()).or_default().push_str(line);
    }
    out
}

fn find_label(line: &str) -> Option<String> {
    let mut rest = line;
    while let Some(i) = rest.find('@') {
        let after = &rest[i + 1..];
        let n = after.find('@')?;
        let lab = &after[..n];
        if lab.len() > 1
            && (lab.starts_with('e') || lab.starts_with('m'))
            && lab[1..].bytes().all(|b| b.is_ascii_digit())
        {
            return Some(lab.to_string());
        }
        rest = after;
    }
    None
}

struct Finding {
    class: String,
    case: String,
    label: String,
    stmt: String,
    java: String,
    engine: String,
    /// Files of a self-contained reproduction.
    repro: Vec<(String, String)>,
}

fn one_line(s: &str) -> String {
    let s = s.replace('\t', "\\t").replace('\n', "\\n");
    if s.chars().count() > 400 {
        s.chars().take(400).collect::<String>() + "..."
    } else {
        s
    }
}

fn campaign(root: &Path, o: &Opts) {
    let start = Instant::now();
    let deadline = o.minutes.map(|m| start + Duration::from_secs_f64(m * 60.0));
    let max_batches = o
        .batches
        .unwrap_or(if deadline.is_some() { usize::MAX } else { 1 });
    let mut rng = Rng::new(o.seed);
    let mut stats: BTreeMap<String, usize> = BTreeMap::new();
    let mut nfind = std::fs::read_dir(o.out.join("findings"))
        .map(|d| d.count())
        .unwrap_or(0);
    let mut findings_tsv = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(o.out.join("findings.tsv"))
        .unwrap_or_else(|e| crate::die(&e.to_string()));
    for b in 0..max_batches {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }
        let bseed = rng.next_u64();
        let bname = format!("b{}-{}", o.seed, b);
        let dir = o.out.join(&bname);
        let _ = std::fs::remove_dir_all(&dir);
        let cases: Vec<GenCase> = (0..o.cases)
            .map(|i| {
                let mut r = Rng::new(bseed.wrapping_add(i as u64 * 7919));
                match o.kind.as_str() {
                    "expr" => {
                        expr_case(&mut r, &format!("x{}_{}_{i}", o.seed, b), o.stmts, o.depth)
                    }
                    _ => prog_case(&mut r, &format!("p{}_{}_{i}", o.seed, b)),
                }
            })
            .collect();
        let cf = write_batch(root, &dir, &cases);
        let t0 = Instant::now();
        let ran = match run_batch(root, &dir, &cf) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("batch {bname}: {e}");
                *stats.entry("batch-errors".into()).or_default() += 1;
                continue;
            }
        };
        let mut findings = Vec::new();
        for c in &cases {
            *stats.entry("cases".into()).or_default() += 1;
            *stats.entry("statements".into()).or_default() += c.stmts.len();
            let Some(r) = ran.get(&c.id) else { continue };
            findings.extend(case_findings(c, r, &mut stats));
        }
        if o.minimize && o.kind == "expr" {
            minimize_expr_findings(root, &dir, &cases, &mut findings);
        }
        for f in &findings {
            nfind += 1;
            let fdir = o
                .out
                .join("findings")
                .join(format!("{nfind:05}-{}", f.class));
            let _ = std::fs::create_dir_all(&fdir);
            for (n, t) in &f.repro {
                let _ = std::fs::write(fdir.join(n), t);
            }
            let _ = std::fs::write(fdir.join("fuzz.ecore"), fz::FUZZ_ECORE);
            let _ = writeln!(
                findings_tsv,
                "{nfind:05}\t{}\t{}\t{}\t{}\t{}\t{}",
                f.class,
                f.case,
                f.label,
                one_line(&f.stmt),
                one_line(&f.java),
                one_line(&f.engine)
            );
            *stats.entry(format!("finding:{}", f.class)).or_default() += 1;
        }
        let _ = findings_tsv.flush();
        if !o.keep {
            let _ = std::fs::remove_dir_all(&dir);
        }
        eprintln!(
            "batch {b} ({:.1} s): {} findings; total {:?} after {:.1} min",
            t0.elapsed().as_secs_f64(),
            findings.len(),
            stats,
            start.elapsed().as_secs_f64() / 60.0
        );
    }
    println!("{stats:?}");
}

fn expr_case(rng: &mut Rng, id: &str, n: usize, depth: u32) -> GenCase {
    let model = fz::model::generate(rng);
    let mut stmts = Vec::new();
    {
        let mut g = Gen::new(rng, expr::base_scope());
        for k in 0..n {
            let d = 1 + (k as u32 % depth.max(1));
            stmts.push(expr::statement(&mut g, &format!("@e{k}@"), d));
        }
    }
    let name = format!("{id}.qvto");
    let text = expr::transformation(id, &stmts);
    GenCase {
        id: id.to_string(),
        files: vec![(name.clone(), text), (format!("{id}.xmi"), model)],
        qvto: name,
        input: Some(format!("{id}.xmi")),
        stmts,
    }
}

fn prog_case(rng: &mut Rng, id: &str) -> GenCase {
    let model = fz::model::generate(rng);
    let text = fz::prog::transformation(rng, id);
    let name = format!("{id}.qvto");
    GenCase {
        id: id.to_string(),
        files: vec![(name.clone(), text), (format!("{id}.xmi"), model)],
        qvto: name,
        input: Some(format!("{id}.xmi")),
        stmts: Vec::new(),
    }
}

/// The findings of one case: a crash, a Java-side failure, different compile problems, or (for
/// expression cases) every statement whose log segment differs, else any other difference.
fn case_findings(c: &GenCase, r: &Ran, stats: &mut BTreeMap<String, usize>) -> Vec<Finding> {
    let repro = c.files.clone();
    let mk = |class: &str, label: &str, stmt: &str, java: String, engine: String| Finding {
        class: class.into(),
        case: c.id.clone(),
        label: label.into(),
        stmt: stmt.into(),
        java,
        engine,
        repro: repro.clone(),
    };
    let Some(j) = &r.java else {
        return vec![mk("java-missing", "-", "", String::new(), String::new())];
    };
    if j.verdict() != "pass" {
        *stats.entry(format!("java-{}", j.verdict())).or_default() += 1;
        if j.verdict() == "timeout" || j.verdict() == "error" {
            return vec![mk(
                "java-error",
                "-",
                "",
                j.get("detail").unwrap_or("").into(),
                String::new(),
            )];
        }
    }
    if let Status::Crashed(m) = &r.outcome.status {
        return vec![mk("crash", "-", "", String::new(), m.clone())];
    }
    if let Some(s) = &r.sema {
        return vec![mk("sema", "-", "", String::new(), s.clone())];
    }
    if has_errors(r) {
        *stats.entry("compile-errors".into()).or_default() += 1;
        return Vec::new();
    }
    let Some(d) = &r.diff else { return Vec::new() };
    if d.equal() {
        if matches!(d.log, Aspect::Unstable) || matches!(d.dump, Aspect::Unstable) {
            *stats.entry("unstable".into()).or_default() += 1;
        }
        return Vec::new();
    }
    let mut out = Vec::new();
    if !c.stmts.is_empty() {
        let js = segments(j.log());
        let es = segments(&r.outcome.log);
        for s in &c.stmts {
            let lab = s.label.trim_matches('@');
            let a = js.get(lab).cloned().unwrap_or_default();
            let b = es.get(lab).cloned().unwrap_or_default();
            if a != b {
                out.push(mk("value", lab, &s.render(), a, b));
            }
        }
        let a = js.get("-").cloned().unwrap_or_default();
        let b = es.get("-").cloned().unwrap_or_default();
        if a != b {
            out.push(mk("log", "-", "", a, b));
        }
    }
    if out.is_empty() || !matches!(d.status, Aspect::Equal) {
        let class = if !matches!(d.status, Aspect::Equal) {
            "status"
        } else if !matches!(d.dump, Aspect::Equal) {
            "model"
        } else if !matches!(d.stack, Aspect::Equal) {
            "stack"
        } else {
            "log"
        };
        out.push(mk(
            class,
            "-",
            "",
            j.log().to_string(),
            d.first_difference().unwrap_or_default(),
        ));
    }
    out
}

/// Minimises the `value` findings of a batch: the finding's statement alone (in a fresh `main`
/// on the case's model), then rounds of reductions.
fn minimize_expr_findings(root: &Path, dir: &Path, cases: &[GenCase], findings: &mut [Finding]) {
    const MAX: usize = 24;
    const CANDIDATES: usize = 120;
    let by_id: BTreeMap<&str, &GenCase> = cases.iter().map(|c| (c.id.as_str(), c)).collect();
    // (finding index, model text, current statement)
    let mut work: Vec<(usize, String, Stmt)> = Vec::new();
    for (i, f) in findings.iter().enumerate() {
        if f.class != "value" || work.len() >= MAX {
            continue;
        }
        let c = by_id[f.case.as_str()];
        let Some(s) = c
            .stmts
            .iter()
            .find(|s| s.label.trim_matches('@') == f.label)
        else {
            continue;
        };
        let model = c.files[1].1.clone();
        work.push((i, model, s.clone()));
    }
    if work.is_empty() {
        return;
    }
    let mdir = dir.join("min");
    // round 0: the statement alone; later rounds: its reductions
    let mut round = 0;
    let mut active: Vec<bool> = vec![true; work.len()];
    let mut isolated: Vec<bool> = vec![false; work.len()];
    while round < 15 && active.iter().any(|a| *a) {
        let mut gen_cases = Vec::new();
        let mut cand: BTreeMap<String, (usize, Stmt)> = BTreeMap::new();
        for (w, (_, model, cur)) in work.iter().enumerate() {
            if !active[w] {
                continue;
            }
            let mut alts: Vec<Stmt> = if round == 0 {
                vec![cur.clone()]
            } else {
                let mut seen = std::collections::BTreeSet::new();
                let mut v: Vec<_> = cur
                    .expr
                    .reductions()
                    .into_iter()
                    .filter(|e| seen.insert(e.render()))
                    .map(|e| Stmt {
                        expr: e,
                        ..cur.clone()
                    })
                    .collect();
                v.sort_by_key(|s| s.expr.size());
                v.truncate(CANDIDATES);
                v
            };
            for (k, s) in alts.drain(..).enumerate() {
                let id = format!("m{round}_{w}_{k}");
                let st = Stmt {
                    label: "@m0@".into(),
                    ..s.clone()
                };
                let text = expr::transformation(&id, std::slice::from_ref(&st));
                gen_cases.push(GenCase {
                    id: id.clone(),
                    files: vec![
                        (format!("{id}.qvto"), text),
                        (format!("{id}.xmi"), model.clone()),
                    ],
                    qvto: format!("{id}.qvto"),
                    input: Some(format!("{id}.xmi")),
                    stmts: vec![st],
                });
                cand.insert(id, (w, s));
            }
        }
        let _ = std::fs::remove_dir_all(&mdir);
        let cf = write_batch(root, &mdir, &gen_cases);
        let Ok(ran) = run_batch(root, &mdir, &cf) else {
            break;
        };
        // per work item, the smallest differing candidate
        let mut best: BTreeMap<usize, (usize, String, Stmt, String, String)> = BTreeMap::new();
        for gc in &gen_cases {
            let (w, s) = &cand[&gc.id];
            let Some(r) = ran.get(&gc.id) else { continue };
            let Some(j) = &r.java else { continue };
            if has_errors(r) || r.sema.is_some() || j.verdict() != "pass" {
                continue;
            }
            let a = segments(j.log()).get("m0").cloned().unwrap_or_default();
            let b = segments(&r.outcome.log)
                .get("m0")
                .cloned()
                .unwrap_or_default();
            let status_differs = r
                .diff
                .as_ref()
                .is_some_and(|d| !matches!(d.status, Aspect::Equal));
            if a != b || status_differs {
                let size = s.expr.size();
                if best.get(w).is_none_or(|b| size < b.0) {
                    best.insert(*w, (size, gc.id.clone(), s.clone(), a, b));
                }
            }
        }
        for w in 0..work.len() {
            if !active[w] {
                continue;
            }
            match best.remove(&w) {
                Some((_, id, s, a, b)) => {
                    if round == 0 {
                        isolated[w] = true;
                    }
                    let gc = gen_cases.iter().find(|g| g.id == id).unwrap();
                    let f = &mut findings[work[w].0];
                    f.stmt = s.render();
                    f.java = a;
                    f.engine = b;
                    f.repro = gc.files.clone();
                    work[w].2 = s;
                }
                None => active[w] = false,
            }
        }
        round += 1;
    }
    for (w, (i, _, _)) in work.iter().enumerate() {
        if !isolated[w] {
            findings[*i].class = "value-context".into();
        }
    }
    let _ = std::fs::remove_dir_all(&mdir);
}
