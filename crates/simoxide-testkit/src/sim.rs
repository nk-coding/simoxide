//! The [`Simulator`] abstraction the differential tests drive, the Java reference implementation
//! ([`RefSim`], `reference/refsim batch` in one warm JVM) and adapters.

use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::runcfg::RunConfig;
use crate::tape::Tape;

/// Where the simulator's random numbers come from.
#[derive(Clone, Debug, Default)]
pub enum RngMode {
    /// Own RNG port, seeded from `run.json`.
    #[default]
    OwnRng,
    /// Replay the reference tape (uniforms in order, origins checkable).
    TapeReplay(Arc<Tape>),
}

impl RngMode {
    pub fn label(&self) -> &'static str {
        match self {
            RngMode::OwnRng => "own-rng",
            RngMode::TapeReplay(_) => "tape-replay",
        }
    }
}

/// One simulation run.
#[derive(Clone, Debug)]
pub struct RunRequest {
    /// Run name (the trace header's `run`; the reference uses the model directory name).
    pub name: String,
    /// Directory with the PCM model files.
    pub model_dir: PathBuf,
    pub config: RunConfig,
    pub rng: RngMode,
    /// Produce `trace.jsonl` and `tape.jsonl` (measurements are always produced).
    pub trace: bool,
}

impl RunRequest {
    /// Request for a model directory with its `run.json`.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<RunRequest, String> {
        let dir = dir.as_ref();
        Ok(RunRequest {
            name: dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "run".into()),
            model_dir: dir.to_path_buf(),
            config: RunConfig::load(dir.join("run.json"))?,
            rng: RngMode::OwnRng,
            trace: true,
        })
    }
}

/// Outputs of one run (the three `palladio-trace/1` files as text).
#[derive(Clone, Debug, Default)]
pub struct RunOutput {
    pub trace: Option<String>,
    pub tape: Option<String>,
    pub measurements: String,
    pub wall: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimError {
    /// The simulator does not exist yet / does not implement this mode.
    Unimplemented,
    /// The simulator declines the model (unsupported feature).
    Unsupported(String),
    /// The run failed (exception, panic, bad model); message and log excerpt.
    Failed(String),
    Timeout(Duration),
    Io(String),
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::Unimplemented => write!(f, "unimplemented"),
            SimError::Unsupported(m) => write!(f, "unsupported: {m}"),
            SimError::Failed(m) => write!(f, "failed: {m}"),
            SimError::Timeout(d) => write!(f, "timeout after {:.1} s", d.as_secs_f64()),
            SimError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for SimError {}

/// A simulator under test (or the reference).
pub trait Simulator {
    fn name(&self) -> &str;
    fn run(&mut self, req: &RunRequest) -> Result<RunOutput, SimError>;
    /// Runs many requests; implementations may batch (the reference amortizes JVM start-up).
    fn run_batch(&mut self, reqs: &[RunRequest]) -> Vec<Result<RunOutput, SimError>> {
        reqs.iter().map(|r| self.run(r)).collect()
    }
}

/// Placeholder until `simoxide-sim` exists: every run is [`SimError::Unimplemented`].
#[derive(Clone, Debug, Default)]
pub struct Unimplemented;

impl Simulator for Unimplemented {
    fn name(&self) -> &str {
        "unimplemented"
    }
    fn run(&mut self, _: &RunRequest) -> Result<RunOutput, SimError> {
        Err(SimError::Unimplemented)
    }
}

/// Adapter: a closure as a [`Simulator`] (panics are caught and reported as failures).
pub struct FnSim<F> {
    name: String,
    f: F,
}

impl<F: FnMut(&RunRequest) -> Result<RunOutput, SimError>> FnSim<F> {
    pub fn new(name: impl Into<String>, f: F) -> Self {
        FnSim {
            name: name.into(),
            f,
        }
    }
}

impl<F: FnMut(&RunRequest) -> Result<RunOutput, SimError>> Simulator for FnSim<F> {
    fn name(&self) -> &str {
        &self.name
    }
    fn run(&mut self, req: &RunRequest) -> Result<RunOutput, SimError> {
        let t0 = Instant::now();
        let f = &mut self.f;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(req))) {
            Ok(Ok(mut o)) => {
                if o.wall.is_zero() {
                    o.wall = t0.elapsed();
                }
                Ok(o)
            }
            Ok(Err(e)) => Err(e),
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                Err(SimError::Failed(format!("panic: {msg}")))
            }
        }
    }
}

/// Workspace root (`simoxide/`).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// The Java reference (`reference/refsim`), run in `batch` mode: one JVM per batch, warm runs.
#[derive(Clone, Debug)]
pub struct RefSim {
    pub refsim: PathBuf,
    /// Scratch directory for staged inputs and outputs (removed after each batch unless `keep`).
    pub work_dir: PathBuf,
    /// Allowed time until the first result line (JVM start, bootstrap, warm-up run).
    pub startup_timeout: Duration,
    /// Allowed time between two result lines.
    pub model_timeout: Duration,
    pub keep: bool,
    /// Extra environment for the JVM (e.g. `JVM_OPTS`).
    pub env: Vec<(String, String)>,
    /// Extra `refsim batch` arguments (e.g. `--repeat 2` for an in-JVM determinism check).
    pub extra_args: Vec<String>,
}

static STAGE: AtomicU64 = AtomicU64::new(0);

impl Default for RefSim {
    fn default() -> Self {
        RefSim {
            refsim: workspace_root().join("reference/refsim"),
            work_dir: std::env::temp_dir().join("testkit-refsim"),
            startup_timeout: Duration::from_secs(120),
            model_timeout: Duration::from_secs(60),
            keep: false,
            env: Vec::new(),
            extra_args: Vec::new(),
        }
    }
}

/// Model files of a directory (everything except `run.json`, `expected/` and other directories).
pub fn model_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|x| {
                    simoxide_model::load::MODEL_EXTENSIONS
                        .iter()
                        .any(|m| x.eq_ignore_ascii_case(m))
                })
        })
        .collect();
    v.sort();
    Ok(v)
}

/// Stages a request's model in `d` (symlinks to the model files + the request's `run.json`), so
/// that the directory name is the run name.
pub fn stage_model(r: &RunRequest, d: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(d)?;
    let src = r.model_dir.canonicalize()?;
    for f in model_files(&src)? {
        let target = d.join(f.file_name().unwrap());
        #[cfg(unix)]
        std::os::unix::fs::symlink(&f, &target)?;
        #[cfg(not(unix))]
        std::fs::copy(&f, &target)?;
    }
    std::fs::write(d.join("run.json"), r.config.to_json())
}

/// The stack trace printed for a failed batch model (its first line equals the status text):
/// the exception line, every `Caused by:` line and the first frames below each.
pub fn stack_excerpt(stderr: &str, status: &str) -> String {
    let head = status.strip_prefix("ERROR ").unwrap_or(status);
    let lines: Vec<&str> = stderr.lines().collect();
    let Some(start) = lines.iter().position(|l| *l == head) else {
        return String::new();
    };
    let mut out = vec![lines[start].to_string()];
    let mut frames = 0;
    for l in &lines[start + 1..] {
        let t = l.trim_start();
        if !(l.starts_with(char::is_whitespace) || t.starts_with("Caused by:")) {
            break;
        }
        if t.starts_with("Caused by:") {
            out.push(t.to_string());
            frames = 0;
        } else if t.starts_with("at ") && frames < 3 {
            out.push(format!("    {t}"));
            frames += 1;
        }
    }
    out.join("\n")
}

#[derive(Debug)]
struct Line {
    name: String,
    status: String,
    /// Wall time of the traced run as reported by refsim (excludes JVM start and warm-up).
    ms: Option<u64>,
}

fn parse_batch_line(l: &str) -> Option<Line> {
    let name = l.split_whitespace().next()?.to_string();
    let pos = l.find(" trace=")?;
    let rest = l[pos + 7..].trim_start();
    let status = rest
        .split_once(char::is_whitespace)
        .map(|(_, s)| s.trim())
        .unwrap_or("")
        .to_string();
    let ms = l
        .find(" ms ")
        .and_then(|e| l[..e].rsplit(char::is_whitespace).next())
        .and_then(|x| x.parse().ok());
    Some(Line { name, status, ms })
}

impl RefSim {
    pub fn new() -> Self {
        Self::default()
    }

    fn stage(&self, reqs: &[&RunRequest]) -> Result<PathBuf, SimError> {
        let id = STAGE.fetch_add(1, Ordering::Relaxed);
        let root = self
            .work_dir
            .join(format!("b{}-{}", std::process::id(), id));
        let io = |e: std::io::Error| SimError::Io(e.to_string());
        let _ = std::fs::remove_dir_all(&root);
        for r in reqs {
            stage_model(r, &root.join("in").join(&r.name)).map_err(io)?;
        }
        Ok(root)
    }

    /// Runs one batch of requests with unique names. Returns results in request order; `None` for
    /// requests not reached (after a timeout).
    fn batch_once(&self, reqs: &[&RunRequest]) -> Vec<Option<Result<RunOutput, SimError>>> {
        let mut res: Vec<Option<Result<RunOutput, SimError>>> =
            (0..reqs.len()).map(|_| None).collect();
        let root = match self.stage(reqs) {
            Ok(r) => r,
            Err(e) => return reqs.iter().map(|_| Some(Err(e.clone()))).collect(),
        };
        let out_dir = root.join("out");
        let err_path = root.join("stderr.log");
        let trace = reqs.iter().any(|r| r.trace);
        let mut cmd = Command::new(&self.refsim);
        cmd.arg("batch")
            .arg(root.join("in"))
            .arg("--out")
            .arg(&out_dir);
        if !trace {
            cmd.arg("--no-trace");
        }
        cmd.args(&self.extra_args);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let stderr = match std::fs::File::create(&err_path) {
            Ok(f) => f,
            Err(e) => {
                return reqs
                    .iter()
                    .map(|_| Some(Err(SimError::Io(e.to_string()))))
                    .collect();
            }
        };
        cmd.stdout(Stdio::piped())
            .stderr(stderr)
            .stdin(Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return reqs
                    .iter()
                    .map(|_| Some(Err(SimError::Io(format!("{}: {e}", self.refsim.display())))))
                    .collect();
            }
        };
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        let mut last = Instant::now();
        let mut first = true;
        let mut timed_out = false;
        let mut lines: Vec<(Line, Duration)> = Vec::new();
        loop {
            let limit = if first {
                self.startup_timeout + self.model_timeout
            } else {
                self.model_timeout
            };
            let left = limit.saturating_sub(last.elapsed());
            match rx.recv_timeout(left.max(Duration::from_millis(1))) {
                Ok(l) => {
                    if let Some(p) = parse_batch_line(&l) {
                        first = false;
                        lines.push((p, last.elapsed()));
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = child.wait();
        let _ = reader.join();
        let log_tail = || {
            let s = std::fs::read_to_string(&err_path).unwrap_or_default();
            let lines: Vec<&str> = s
                .lines()
                .filter(|l| {
                    !l.starts_with("[refsim] bootstrap") && !l.starts_with("[refsim] warm-up")
                })
                .collect();
            lines[lines.len().saturating_sub(25)..].join("\n")
        };
        // SimuLizar runs one Java thread per simulated process: when the JVM cannot create a
        // thread (process or address-space limits, memory), the simulation may end early and
        // still report success. Such a batch's results cannot be trusted.
        let jvm_trouble = std::fs::read_to_string(&err_path)
            .map(|s| {
                s.contains("pthread_create failed")
                    || s.contains("unable to create native thread")
                    || s.contains("OutOfMemoryError")
            })
            .unwrap_or(false);
        let mut reached = 0;
        for (line, wall) in &lines {
            let Some(i) = reqs.iter().position(|r| r.name == line.name) else {
                continue;
            };
            reached = reached.max(i + 1);
            let r = if line.status == "ok" && jvm_trouble {
                Err(SimError::Failed(
                    "refsim JVM could not create a thread or ran out of memory during this batch; result not trusted".into(),
                ))
            } else if line.status == "ok" {
                let d = out_dir.join(&line.name);
                let rd = |f: &str| std::fs::read_to_string(d.join(f));
                match rd("measurements.csv") {
                    Ok(m) => Ok(RunOutput {
                        trace: if trace { rd("trace.jsonl").ok() } else { None },
                        tape: if trace { rd("tape.jsonl").ok() } else { None },
                        measurements: m,
                        wall: line.ms.map(Duration::from_millis).unwrap_or(*wall),
                    }),
                    Err(e) => Err(SimError::Io(e.to_string())),
                }
            } else {
                let log = std::fs::read_to_string(&err_path).unwrap_or_default();
                Err(SimError::Failed(format!(
                    "{}\n{}",
                    line.status,
                    stack_excerpt(&log, &line.status)
                )))
            };
            res[i] = Some(r);
        }
        if timed_out {
            // the first request without a result line is the one that hung
            if let Some(i) = (0..reqs.len()).find(|&i| res[i].is_none()) {
                res[i] = Some(Err(SimError::Timeout(self.model_timeout)));
            }
        } else {
            let tail = log_tail();
            for r in res.iter_mut().skip(reached) {
                if r.is_none() {
                    *r = Some(Err(SimError::Failed(format!(
                        "refsim exited early\n{tail}"
                    ))));
                }
            }
        }
        if !self.keep {
            let _ = std::fs::remove_dir_all(&root);
        }
        res
    }
}

impl Simulator for RefSim {
    fn name(&self) -> &str {
        "refsim"
    }
    fn run(&mut self, req: &RunRequest) -> Result<RunOutput, SimError> {
        self.run_batch(std::slice::from_ref(req)).pop().unwrap()
    }
    /// Runs in as few JVMs as possible (the batch runs models in name order, so requests are
    /// grouped by unique name and sorted like refsim sorts directories).
    fn run_batch(&mut self, reqs: &[RunRequest]) -> Vec<Result<RunOutput, SimError>> {
        let mut out: Vec<Option<Result<RunOutput, SimError>>> =
            (0..reqs.len()).map(|_| None).collect();
        let mut pending: Vec<usize> = (0..reqs.len()).collect();
        while !pending.is_empty() {
            // one group: first occurrence of every name
            let mut seen = std::collections::HashSet::new();
            let mut group: Vec<usize> = Vec::new();
            let mut rest = Vec::new();
            for &i in &pending {
                if seen.insert(reqs[i].name.clone()) {
                    group.push(i);
                } else {
                    rest.push(i);
                }
            }
            // refsim batch sorts model directories by name (File.compareTo)
            group.sort_by(|&a, &b| reqs[a].name.cmp(&reqs[b].name));
            let mut todo = group;
            while !todo.is_empty() {
                let refs: Vec<&RunRequest> = todo.iter().map(|&i| &reqs[i]).collect();
                let res = self.batch_once(&refs);
                let mut next = Vec::new();
                for (k, r) in res.into_iter().enumerate() {
                    match r {
                        Some(r) => out[todo[k]] = Some(r),
                        None => next.push(todo[k]),
                    }
                }
                if next.len() == todo.len() {
                    for i in next {
                        out[i] = Some(Err(SimError::Failed("refsim produced no result".into())));
                    }
                    break;
                }
                todo = next;
            }
            pending = rest;
        }
        out.into_iter().map(|r| r.unwrap()).collect()
    }
}

/// A simulator behind a command line (e.g. the future `simoxide` CLI). The template's arguments may
/// contain the placeholders `{dir}` (staged model directory named after the run, with `run.json`),
/// `{run_json}`, `{trace}`, `{tape}`,
/// `{measurements}` (output files to write), `{mode}` (`own-rng` / `tape-replay`), `{tape_in}` (the
/// reference tape to replay; empty in own-RNG mode), `{seed}`, `{name}`. An argument
/// `{replay:FLAG}` becomes `FLAG <tape_in>` in replay mode and disappears in own-RNG mode, e.g.
/// `simoxide run --model {dir} --run-json {run_json} --name {name} --trace {trace} --tape {tape}
/// --measurements {measurements} {replay:--replay-tape}`.
#[derive(Clone, Debug)]
pub struct CmdSim {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub work_dir: PathBuf,
    pub timeout: Duration,
}

impl CmdSim {
    /// `spec` = `program arg1 arg2 ...` (whitespace separated).
    pub fn parse(spec: &str) -> Result<CmdSim, String> {
        let mut it = spec.split_whitespace();
        let program = it.next().ok_or("empty command")?;
        Ok(CmdSim {
            name: Path::new(program)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| program.to_string()),
            program: program.into(),
            args: it.map(str::to_string).collect(),
            work_dir: std::env::temp_dir().join("testkit-cmdsim"),
            timeout: Duration::from_secs(120),
        })
    }
}

impl Simulator for CmdSim {
    fn name(&self) -> &str {
        &self.name
    }
    fn run(&mut self, req: &RunRequest) -> Result<RunOutput, SimError> {
        let id = STAGE.fetch_add(1, Ordering::Relaxed);
        let d = self
            .work_dir
            .join(format!("c{}-{}", std::process::id(), id));
        let io = |e: std::io::Error| SimError::Io(e.to_string());
        let md = d.join(&req.name);
        stage_model(req, &md).map_err(io)?;
        let run_json = md.join("run.json");
        let tape_in = match &req.rng {
            RngMode::TapeReplay(t) => {
                let p = d.join("tape-in.jsonl");
                std::fs::write(&p, t.to_text()).map_err(io)?;
                p.display().to_string()
            }
            RngMode::OwnRng => String::new(),
        };
        let (trace, tape, meas) = (
            d.join("trace.jsonl"),
            d.join("tape.jsonl"),
            d.join("measurements.csv"),
        );
        let subst = |a: &str| {
            a.replace("{dir}", &md.display().to_string())
                .replace("{run_json}", &run_json.display().to_string())
                .replace("{trace}", &trace.display().to_string())
                .replace("{tape}", &tape.display().to_string())
                .replace("{measurements}", &meas.display().to_string())
                .replace("{mode}", req.rng.label())
                .replace("{tape_in}", &tape_in)
                .replace("{seed}", &req.config.seed.to_string())
                .replace("{name}", &req.name)
        };
        let t0 = Instant::now();
        let mut child = Command::new(&self.program)
            .args(self.args.iter().flat_map(|a| {
                match a.strip_prefix("{replay:").and_then(|x| x.strip_suffix('}')) {
                    Some(flag) if !tape_in.is_empty() => vec![flag.to_string(), tape_in.clone()],
                    Some(_) => vec![],
                    None => vec![subst(a)],
                }
            }))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SimError::Io(format!("{}: {e}", self.program.display())))?;
        let mut stderr = child.stderr.take().unwrap();
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
            s
        });
        let status = loop {
            if let Some(st) = child.try_wait().map_err(io)? {
                break st;
            }
            if t0.elapsed() > self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&d);
                return Err(SimError::Timeout(self.timeout));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let wall = t0.elapsed();
        let err = errs.join().unwrap_or_default();
        let res = if status.success() {
            match std::fs::read_to_string(&meas) {
                Ok(m) => Ok(RunOutput {
                    trace: std::fs::read_to_string(&trace).ok(),
                    tape: std::fs::read_to_string(&tape).ok(),
                    measurements: m,
                    wall,
                }),
                Err(e) => Err(SimError::Failed(format!(
                    "no measurements written: {e}\n{err}"
                ))),
            }
        } else if status.code() == Some(3) {
            Err(SimError::Unsupported(
                err.lines().last().unwrap_or("").to_string(),
            ))
        } else {
            let tail: Vec<&str> = err.lines().collect();
            Err(SimError::Failed(format!(
                "exit {status}\n{}",
                tail[tail.len().saturating_sub(20)..].join("\n")
            )))
        };
        let _ = std::fs::remove_dir_all(&d);
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_line() {
        let l = "h01_ps_single                            3f1a2b3c    272 ms  t_end=30.1           uniforms=204      meas=606     trace=118000    ok";
        let p = parse_batch_line(l).unwrap();
        assert_eq!(p.name, "h01_ps_single");
        assert_eq!(p.status, "ok");
        assert_eq!(p.ms, Some(272));
        let l = "gen_1 -              0 ms  t_end=-              uniforms=0        meas=0       trace=0         ERROR java.lang.RuntimeException: x y";
        let p = parse_batch_line(l).unwrap();
        assert_eq!(p.status, "ERROR java.lang.RuntimeException: x y");
        assert!(parse_batch_line("ALL OK").is_none());
    }

    #[test]
    fn excerpt() {
        let err = "noise\njava.lang.RuntimeException: boom\n\tat a.b(C.java:1)\n\tat a.c(C.java:2)\n\tat a.d(C.java:3)\n\tat a.e(C.java:4)\nCaused by: java.lang.NullPointerException: x\n\tat q.r(S.java:9)\n\t... 5 more\n[refsim] next\n";
        let e = stack_excerpt(err, "ERROR java.lang.RuntimeException: boom");
        assert_eq!(
            e,
            "java.lang.RuntimeException: boom\n    at a.b(C.java:1)\n    at a.c(C.java:2)\n    at a.d(C.java:3)\nCaused by: java.lang.NullPointerException: x\n    at q.r(S.java:9)"
        );
    }

    #[test]
    fn fn_sim_catches_panics() {
        let mut s = FnSim::new("p", |_: &RunRequest| -> Result<RunOutput, SimError> {
            panic!("boom")
        });
        let req = RunRequest {
            name: "x".into(),
            model_dir: ".".into(),
            config: RunConfig::default(),
            rng: RngMode::OwnRng,
            trace: false,
        };
        assert_eq!(
            s.run(&req).unwrap_err(),
            SimError::Failed("panic: boom".into())
        );
        assert_eq!(
            Unimplemented.run(&req).unwrap_err(),
            SimError::Unimplemented
        );
    }
}
