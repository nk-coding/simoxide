//! Running the Java reference runner on case files, recording its results as fixtures, and
//! replaying its results as an [`Engine`] (to check the harness itself).

use crate::case::Case;
use crate::engine::{CaseEnv, Engine, Outcome, Outputs, Severity, Status};
use crate::fixture::{self, JavaResult};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Options of a Java batch run.
#[derive(Clone, Debug, Default)]
pub struct JavaRun {
    /// Repository root (the case files' base).
    pub base: PathBuf,
    pub cases: PathBuf,
    pub out: PathBuf,
    pub only: Vec<String>,
    pub prefixes: Vec<String>,
    pub tape: bool,
    pub save_xmi: bool,
    /// Extra JVM options (`JVM_OPTS`), e.g. `-XX:+UnlockExperimentalVMOptions -XX:hashCode=3`.
    pub jvm_opts: Option<String>,
}

impl JavaRun {
    /// Runs `reference/runner/qvto-run --batch` (builds the runner first if needed). Returns the
    /// runner's console output.
    pub fn run(&self) -> Result<String, String> {
        let runner = self.base.join("reference/runner");
        if !runner.join("build/classpath.txt").is_file() {
            let st = Command::new(runner.join("build.sh")).status().map_err(|e| format!("build.sh: {e}"))?;
            if !st.success() {
                return Err("reference/runner/build.sh failed".into());
            }
        }
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
        cmd.env("XMX", "-Xmx3g");
        if let Some(o) = &self.jvm_opts {
            cmd.env("JVM_OPTS", o);
        }
        let out = cmd.output().map_err(|e| format!("qvto-run: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
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
    let mut out = BTreeMap::new();
    for (id, r) in first {
        let Some(s) = second.get(id) else { continue };
        let mut u = Vec::new();
        for (f, name) in [("out.jsonl", "dump"), ("log.txt", "log"), ("stack.txt", "stack")] {
            if r.file(f) != s.file(f) {
                u.push(name);
            }
        }
        if !u.is_empty() {
            out.insert(id.clone(), u);
        }
    }
    out
}

/// Records Java results as a fixture: runs the batch `runs` times (the extra runs without tape;
/// every other one with constant identity hashes, `-XX:hashCode=2`, which turns any
/// identity-hash-dependent order into another order), marks the outputs that differ in any
/// rerun as unstable, writes the bundle. The first run's
/// directory stays as the cache (with tapes and XMI).
pub fn record(run: &JavaRun, bundle: &Path, runs: usize) -> Result<String, String> {
    let mut log = run.run()?;
    let mut a = fixture::read_result_dir(&run.out).map_err(|e| e.to_string())?;
    let mut flagged = BTreeMap::new();
    for k in 1..runs.max(1) {
        let mut again = run.clone();
        again.out = run.out.with_extension(format!("rerun{k}"));
        again.tape = false;
        again.save_xmi = false;
        if k % 2 == 1 {
            // constant identity hashes: identity-hash-dependent orders become insertion orders
            again.jvm_opts = Some("-XX:+UnlockExperimentalVMOptions -XX:hashCode=2".into());
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
            r.files.entry("result.txt".into()).or_default().push_str(&format!("unstable\t{}\n", u.join(",")));
        }
    }
    if let Some(d) = bundle.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(bundle, fixture::write_bundle(&a)).map_err(|e| e.to_string())?;
    log.push_str(&format!(
        "{} cases recorded in {} ({runs} runs), {} with unstable outputs\n",
        a.len(),
        bundle.display(),
        flagged.len()
    ));
    Ok(log)
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
        Ok(JavaReplay { dir: dir.to_path_buf(), results: fixture::read_result_dir(dir)? })
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
    o.run_time = r.get("run-ms").and_then(|t| t.parse::<f64>().ok()).map(|ms| Duration::from_secs_f64(ms / 1000.0));
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
            Err(e) => return Outcome::with_status(Status::Crashed(format!("{}: {e}", p.display()))),
        }
    }
    rs.resolve_all();
    o.outputs = Some(Outputs { rs, extents });
    o
}

/// The cases of a case file that the Java runner runs (runnable ones).
pub fn runnable(cases: &[Case]) -> impl Iterator<Item = &Case> {
    cases.iter().filter(|c| c.runnable())
}
