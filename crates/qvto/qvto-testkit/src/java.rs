//! Running the Java reference runner on case files, recording its results as fixtures, and
//! replaying its results as an [`Engine`] (to check the harness itself).

use crate::case::Case;
use crate::engine::{CaseEnv, Engine, Outcome, Outputs, Severity, Status};
use crate::fixture::{self, JavaResult};
use simoxide_oraclekit::batch::{self, Deadlines, Stderr};
use simoxide_oraclekit::{java, results};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Options of a Java batch run.
#[derive(Clone, Debug, Default)]
pub struct JavaRun {
    /// The QVTo root (the case files' base).
    pub base: PathBuf,
    pub cases: PathBuf,
    pub out: PathBuf,
    pub only: Vec<String>,
    pub prefixes: Vec<String>,
    pub tape: bool,
    pub save_xmi: bool,
    /// Extra JVM options (`JVM_OPTS`), e.g. `-XX:+UnlockExperimentalVMOptions -XX:hashCode=3`.
    pub jvm_opts: Option<String>,
    /// Per-case deadline in seconds (the runner's default: 120).
    pub timeout: Option<u64>,
    /// The `det` runner: the shared seed agent with its ordering rule
    /// (`-javaagent:seed-agent.jar=ordered,qvto`, [`det_agent`]; the Slingshot runner's `det`
    /// mode), so that QVTo's and OCL's `HashSet`/`HashMap` iterate in insertion order.
    pub det: bool,
}

/// The seed agent with its ordering rule (the jar of `simoxide_oraclekit::java::seed_agent`,
/// built from `reference/oraclekit/seed-agent` at the repository root, two levels above the
/// QVTo root, if missing); `QVTO_DET_AGENT` names another jar.
pub fn det_agent(base: &Path) -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("QVTO_DET_AGENT") {
        return Ok(PathBuf::from(p));
    }
    java::seed_agent(&qvto_paths::workspace_root(base))
}

impl JavaRun {
    /// Runs `reference/runner/qvto-run --batch` (builds the runner first if needed) as an
    /// oraclekit batch process. Returns the runner's console output (standard output, then
    /// standard error).
    pub fn run(&self) -> Result<String, String> {
        let runner = self.base.join("reference/runner");
        java::ensure_built(
            &runner.join("build.sh"),
            &[&runner.join("build/classpath.txt")],
        )?;
        let mut cmd = Command::new(runner.join("qvto-run"));
        cmd.current_dir(&self.base)
            .arg("--batch")
            .arg(&self.cases)
            .arg("--out")
            .arg(&self.out)
            .arg("--base")
            .arg(&self.base);
        if !self.only.is_empty() {
            cmd.arg("--only").arg(self.only.join(","));
        }
        for p in &self.prefixes {
            cmd.arg("--prefix").arg(p);
        }
        if !self.tape {
            cmd.arg("--no-tape");
        }
        if self.save_xmi {
            cmd.arg("--save-xmi");
        }
        if let Some(t) = self.timeout {
            cmd.arg("--timeout").arg(t.to_string());
        }
        cmd.env("XMX", "-Xmx2g");
        let mut opts: Vec<String> = Vec::new();
        if self.det {
            let jar = det_agent(&self.base)?;
            opts.push(java::agent_option(&jar, java::ORDERED_QVTO));
        }
        opts.extend(self.jvm_opts.clone());
        if !opts.is_empty() {
            cmd.env("JVM_OPTS", opts.join(" "));
        }
        // the runner enforces its own per-case deadline (`--timeout`)
        let out = batch::run(&mut cmd, Stderr::Capture, Deadlines::NONE, |_| true)
            .map_err(|e| format!("qvto-run: {e}"))?;
        let text = out.stdout() + &out.stderr;
        if !out.success() {
            return Err(format!("qvto-run failed:\n{text}"));
        }
        Ok(text)
    }
}

/// The outputs (`dump`, `log`, `stack`) of each case that differ between two runs.
pub fn unstable_outputs(
    first: &BTreeMap<String, JavaResult>,
    second: &BTreeMap<String, JavaResult>,
) -> BTreeMap<String, Vec<&'static str>> {
    let label = |f: &str| match f {
        "out.jsonl" => "dump",
        "log.txt" => "log",
        _ => "stack",
    };
    results::differing_files(first, second, &["out.jsonl", "log.txt", "stack.txt"])
        .into_iter()
        .map(|(id, fs)| (id, fs.into_iter().map(label).collect()))
        .collect()
}

/// The outputs of each result that a fixture check compares: without the files the result marks
/// unstable (`dump`: `out.jsonl`, `log`: `log.txt`, `stack`: `stack.txt`) and, when the log is
/// unstable, without the execution diagnostic's lines in `diag.txt` (`exec`, they repeat the log).
pub fn stable_part(results: &BTreeMap<String, JavaResult>) -> results::ResultSet {
    results
        .iter()
        .map(|(id, r)| {
            let unstable = r.unstable();
            let mut files = r.files.clone();
            for (label, f) in [
                ("dump", "out.jsonl"),
                ("log", "log.txt"),
                ("stack", "stack.txt"),
            ] {
                if unstable.contains(&label) {
                    files.remove(f);
                }
            }
            if unstable.contains(&"log")
                && let Some(d) = files.get_mut("diag.txt")
            {
                *d = d
                    .lines()
                    .filter(|l| !l.starts_with("exec\t"))
                    .map(|l| format!("{l}\n"))
                    .collect();
            }
            (id.clone(), files)
        })
        .collect()
}

/// What [`record`] does with the new bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    /// Write it (the fixture file is replaced).
    Write,
    /// Compare it with the committed fixture and write nothing.
    Check,
}

/// The result of [`record`].
#[derive(Debug)]
pub struct Recorded {
    /// The runner's console output and a summary line.
    pub log: String,
    /// With [`Write::Check`]: how the new bundle differs from the committed one in its stable
    /// outputs ([`stable_part`]; empty: unchanged).
    pub differences: Vec<results::Difference>,
}

/// Records Java results as a fixture: runs the batch `runs` times (the extra runs without tape,
/// cycling through constant identity hashes `-XX:hashCode=2`, sequential ones `-XX:hashCode=3`
/// and the default, so that identity-hash-dependent orders change), marks the outputs that differ in any
/// rerun as unstable, writes the bundle (or, with [`Write::Check`], compares it with the
/// committed one). The first run's
/// directory stays as the cache (with tapes and XMI).
/// With `merge`, the recorded cases replace theirs in an existing bundle (a recording of some
/// cases, `--only`/`--prefix`).
pub fn record(
    run: &JavaRun,
    bundle: &Path,
    runs: usize,
    merge: bool,
    write: Write,
) -> Result<Recorded, String> {
    let mut log = run.run()?;
    let mut a = fixture::read_result_dir(&run.out).map_err(|e| e.to_string())?;
    let mut flagged = BTreeMap::new();
    for k in 1..runs.max(1) {
        let mut again = run.clone();
        again.out = run.out.with_extension(format!("rerun{k}"));
        again.tape = false;
        again.save_xmi = false;
        // other identity-hash generators: constant (orders become insertion orders) and a
        // global counter, alternating with HotSpot's default
        match k % 3 {
            1 => again.jvm_opts = Some("-XX:+UnlockExperimentalVMOptions -XX:hashCode=2".into()),
            2 => again.jvm_opts = Some("-XX:+UnlockExperimentalVMOptions -XX:hashCode=3".into()),
            _ => {}
        }
        log.push_str(&again.run()?);
        let b = fixture::read_result_dir(&again.out).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&again.out);
        for (id, u) in unstable_outputs(&a, &b) {
            let e: &mut Vec<&str> = flagged.entry(id).or_default();
            for x in u {
                if !e.contains(&x) {
                    e.push(x);
                }
            }
        }
    }
    for (id, u) in &flagged {
        if let Some(r) = a.get_mut(id) {
            r.files
                .entry("result.txt".into())
                .or_default()
                .push_str(&format!("unstable\t{}\n", u.join(",")));
        }
    }
    let n_recorded = a.len();
    let old = std::fs::read_to_string(bundle).ok();
    if merge && let Some(text) = &old {
        // a partial recording replaces its cases in the existing bundle
        let mut all = fixture::read_bundle(text)?;
        all.extend(std::mem::take(&mut a));
        a = all;
    }
    let text = fixture::write_bundle(&a);
    let mut differences = Vec::new();
    let mut unstable_skipped = 0;
    let what = match write {
        Write::Write => {
            if let Some(d) = bundle.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::write(bundle, &text).map_err(|e| e.to_string())?;
            "recorded in"
        }
        Write::Check => {
            if old.as_deref() != Some(text.as_str()) {
                // both as bundles (the files and lines a bundle keeps), without what each marks
                // unstable: that is one run's identity hashes or timings
                let committed = match &old {
                    Some(t) => fixture::read_bundle(t)?,
                    None => BTreeMap::new(),
                };
                let new = fixture::read_bundle(&text)?;
                let all = results::differences(&committed, &new);
                differences = results::differences(&stable_part(&committed), &stable_part(&new));
                unstable_skipped = all.len() - differences.len();
                if all.is_empty() {
                    // same cases and files, different bytes (an older bundle layout)
                    differences.push(results::Difference::File {
                        case: "-".into(),
                        file: "bundle text".into(),
                    });
                }
            }
            "checked against"
        }
    };
    log.push_str(&format!(
        "{} cases {what} {} ({runs} runs), {} with unstable outputs{}\n",
        n_recorded,
        bundle.display(),
        flagged.len(),
        match write {
            Write::Write => String::new(),
            Write::Check if differences.is_empty() && unstable_skipped == 0 => {
                ": fixture unchanged".into()
            }
            Write::Check if differences.is_empty() => format!(
                ": fixture unchanged in its stable outputs ({unstable_skipped} unstable outputs differ, not compared)"
            ),
            Write::Check => format!(": {} differences", differences.len()),
        }
    ));
    Ok(Recorded { log, differences })
}

/// An engine that replays the Java runner's results (a result directory written with
/// `--save-xmi`): outputs are the saved extents, loaded with qvto-emf. Running the harness with
/// it checks the case model, the Rust comparisons and the fixtures against Java's own verdicts.
pub struct JavaReplay {
    pub dir: PathBuf,
    pub results: BTreeMap<String, JavaResult>,
}

impl JavaReplay {
    pub fn new(dir: &Path) -> std::io::Result<JavaReplay> {
        Ok(JavaReplay {
            dir: dir.to_path_buf(),
            results: fixture::read_result_dir(dir)?,
        })
    }
}

impl Engine for JavaReplay {
    fn name(&self) -> &str {
        "java-replay"
    }

    fn run(&mut self, env: &CaseEnv) -> Outcome {
        let Some(r) = self.results.get(&env.case.id) else {
            return Outcome::with_status(Status::NotImplemented("no Java result".into()));
        };
        replay(env, r, &self.dir.join(&env.case.id))
    }
}

fn replay(env: &CaseEnv, r: &JavaResult, dir: &Path) -> Outcome {
    let mut o = Outcome::with_status(Status::Completed);
    o.load = r.get("load").and_then(Severity::from_name);
    o.problems = r.problems();
    o.severity = r.severity().and_then(Severity::from_name);
    o.code = r.code().unwrap_or(0);
    o.message = r.get("message").unwrap_or("").to_string();
    o.exception = r.exception().map(String::from);
    o.stack = r.stack().map(String::from);
    o.log = r.log().to_string();
    o.trace_records = r.get("trace-records").and_then(|t| t.parse().ok());
    o.run_time = r
        .get("run-ms")
        .and_then(|t| t.parse::<f64>().ok())
        .map(|ms| Duration::from_secs_f64(ms / 1000.0));
    if env.case.compile_only() {
        return o;
    }
    let params = r.get("params").unwrap_or("");
    let out_indexes: Vec<usize> = params
        .split(',')
        .enumerate()
        .filter(|(_, p)| !p.is_empty() && !p.starts_with("in:"))
        .map(|(i, _)| i)
        .collect();
    let mut rs = match env.resource_set() {
        Ok(rs) => rs,
        Err(e) => return Outcome::with_status(Status::Crashed(e)),
    };
    let mut extents = Vec::new();
    for i in out_indexes {
        let p = dir.join(format!("extent{i}.xmi"));
        match rs.load_file(&p) {
            Ok(x) => extents.push(x),
            Err(e) => {
                return Outcome::with_status(Status::Crashed(format!("{}: {e}", p.display())));
            }
        }
    }
    if env.case.dump_models {
        // the model resources as the run left them (written by --save-xmi)
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("inputs"))
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for f in files {
            if let Err(e) = rs.load_file(&f) {
                return Outcome::with_status(Status::Crashed(format!("{}: {e}", f.display())));
            }
        }
    }
    rs.resolve_all();
    let models = if env.case.dump_models {
        Outputs::collect_models(&rs, &extents, &[])
    } else {
        Vec::new()
    };
    o.outputs = Some(Outputs {
        rs,
        extents,
        models,
    });
    o
}

/// The cases of a case file that the Java runner runs (runnable ones).
pub fn runnable(cases: &[Case]) -> impl Iterator<Item = &Case> {
    cases.iter().filter(|c| c.runnable())
}
