//! A warm-JVM batch process: one JVM runs many cases and prints one result line per case on
//! standard output (refsim's `batch`, the QVTo runner's `--batch`).
//!
//! [`run`] spawns the process, reads standard output line by line on a thread, and waits for
//! each result line with a deadline: the first one may take [`Deadlines::startup`] longer (JVM
//! start, bootstrap, warm-up). When a deadline passes the process is killed and
//! [`Outcome::timed_out`] is set; the caller decides which case hung (the first one without a
//! result line) and whether to run the rest again. Standard error goes to a file (kept with the
//! batch's scratch directory) or is captured; either way [`Outcome::stderr`] has it.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Where the process's standard error goes.
#[derive(Clone, Debug)]
pub enum Stderr {
    /// To this file (created or truncated), read back when the process has ended.
    File(PathBuf),
    /// Into memory.
    Capture,
}

/// How long [`run`] waits for result lines.
#[derive(Clone, Copy, Debug, Default)]
pub struct Deadlines {
    /// Extra time for the first result line (JVM start, bootstrap, warm-up run).
    pub startup: Duration,
    /// Time allowed between two result lines (and, plus `startup`, until the first); `None`
    /// waits as long as the process runs.
    pub per_result: Option<Duration>,
}

impl Deadlines {
    /// No deadline: wait until the process closes its standard output.
    pub const NONE: Deadlines = Deadlines {
        startup: Duration::ZERO,
        per_result: None,
    };
}

/// One line of standard output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    /// A result line (the caller's `is_result` said so).
    pub result: bool,
    /// Time since the previous result line (or the start) when the line arrived.
    pub wait: Duration,
}

/// What a batch process did.
#[derive(Debug)]
pub struct Outcome {
    /// Every line of standard output in order.
    pub lines: Vec<Line>,
    /// A deadline passed and the process was killed.
    pub timed_out: bool,
    /// The exit status (`None` if waiting for it failed).
    pub status: Option<ExitStatus>,
    /// Standard error (lossy UTF-8).
    pub stderr: String,
}

impl Outcome {
    /// The result lines.
    pub fn results(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter().filter(|l| l.result)
    }

    /// Standard output as text (each line followed by `\n`).
    pub fn stdout(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(&l.text);
            s.push('\n');
        }
        s
    }

    /// The process ended by itself with exit code 0.
    pub fn success(&self) -> bool {
        !self.timed_out && self.status.is_some_and(|s| s.success())
    }
}

/// Runs `cmd` as a batch process (standard input closed, standard output read here, standard
/// error as `stderr` says). `is_result` marks the result lines; the deadlines apply to them.
/// Errors: the standard error file cannot be created, or the process cannot be started (the
/// message then names the program).
pub fn run(
    cmd: &mut Command,
    stderr: Stderr,
    deadlines: Deadlines,
    mut is_result: impl FnMut(&str) -> bool,
) -> std::io::Result<Outcome> {
    match &stderr {
        Stderr::File(p) => cmd.stderr(std::fs::File::create(p)?),
        Stderr::Capture => cmd.stderr(Stdio::piped()),
    };
    cmd.stdout(Stdio::piped()).stdin(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("{}: {e}", std::path::Path::new(cmd.get_program()).display()),
        )
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for l in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let errs = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = e.read_to_end(&mut b);
            String::from_utf8_lossy(&b).into_owned()
        })
    });
    let mut lines = Vec::new();
    let mut last = Instant::now();
    let mut first = true;
    let mut timed_out = false;
    loop {
        let got = match deadlines.per_result {
            None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            Some(per) => {
                let limit = if first { deadlines.startup + per } else { per };
                let left = limit.saturating_sub(last.elapsed());
                rx.recv_timeout(left.max(Duration::from_millis(1)))
            }
        };
        match got {
            Ok(text) => {
                let result = is_result(&text);
                let wait = last.elapsed();
                if result {
                    first = false;
                    last = Instant::now();
                }
                lines.push(Line { text, result, wait });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                timed_out = true;
                let _ = child.kill();
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().ok();
    let _ = reader.join();
    let stderr = match (&stderr, errs) {
        (_, Some(t)) => t.join().unwrap_or_default(),
        (Stderr::File(p), None) => std::fs::read(p)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default(),
        (Stderr::Capture, None) => String::new(),
    };
    Ok(Outcome {
        lines,
        timed_out,
        status,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    fn result_lines_and_stderr() {
        let o = run(
            &mut sh("echo noise; echo 'r a'; echo oops >&2; echo 'r b'; exit 3"),
            Stderr::Capture,
            Deadlines::NONE,
            |l| l.starts_with("r "),
        )
        .unwrap();
        let r: Vec<&str> = o.results().map(|l| l.text.as_str()).collect();
        assert_eq!(r, ["r a", "r b"]);
        assert_eq!(o.stdout(), "noise\nr a\nr b\n");
        assert_eq!(o.stderr, "oops\n");
        assert!(!o.timed_out);
        assert_eq!(o.status.and_then(|s| s.code()), Some(3));
        assert!(!o.success());
    }

    #[test]
    fn stderr_to_file() {
        let d = std::env::temp_dir().join(format!("oraclekit-batch-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("stderr.log");
        let o = run(
            &mut sh("echo x >&2; echo r"),
            Stderr::File(f.clone()),
            Deadlines::NONE,
            |_| true,
        )
        .unwrap();
        assert!(o.success());
        assert_eq!(o.stderr, "x\n");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "x\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn deadline_kills_a_hanging_process() {
        let t0 = Instant::now();
        let o = run(
            &mut sh("echo 'r a'; exec sleep 30"),
            Stderr::Capture,
            Deadlines {
                startup: Duration::from_millis(200),
                per_result: Some(Duration::from_millis(300)),
            },
            |l| l.starts_with("r "),
        )
        .unwrap();
        assert!(o.timed_out);
        assert_eq!(o.results().count(), 1);
        assert!(t0.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn missing_program_is_named() {
        let e = run(
            &mut Command::new("/nonexistent/oraclekit-tool"),
            Stderr::Capture,
            Deadlines::NONE,
            |_| true,
        )
        .unwrap_err();
        assert!(
            e.to_string().starts_with("/nonexistent/oraclekit-tool: "),
            "{e}"
        );
    }
}
