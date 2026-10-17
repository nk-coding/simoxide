//! JVM conventions shared by the reference harnesses.
//!
//! - The **seed agent** (`reference/oraclekit/seed-agent/`, a `-javaagent`): seeds `Math.random()`
//!   and UUIDs and, with the argument `ordered`, rewrites hash-ordered collections to insertion
//!   order at class-load time (`ordered,qvto` extends the rule to Eclipse QVTo and OCL). refsim
//!   (patch P10), the Slingshot runner's `det` mode and the QVTo runner's `--det` recordings load
//!   it. [`seed_agent`] finds or builds the jar; [`agent_option`] writes the JVM option.
//! - **Building on demand**: [`ensure_built`] runs a runner's `build.sh` when its build output is
//!   missing.
//! - **Reading standard error**: [`stack_excerpt`] cuts a Java stack trace down to its causes;
//!   [`jvm_trouble`] spots a JVM that could not create a thread or ran out of memory (its results
//!   cannot be trusted).
//!
//! The launch itself (classpath, `nice`, heap size, agents) is in the harnesses' shell launchers,
//! which share `reference/oraclekit/lib.sh`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The seed agent's argument for the `det` ordering rule over Slingshot's, QVTo's and OCL's
/// classes (`HashSet`/`HashMap` become `LinkedHashSet`/`LinkedHashMap`).
pub const ORDERED_QVTO: &str = "ordered,qvto";

/// The script that builds the seed agent (`reference/oraclekit/seed-agent.sh [OUT_DIR] [ASM_JAR]`).
pub const SEED_AGENT_SCRIPT: &str = "reference/oraclekit/seed-agent.sh";

/// Where [`seed_agent`] builds the jar by default (relative to the repository root); refsim and
/// the Slingshot runner build their own copies next to their other build output, each with the
/// ASM jar of the product it runs.
pub const SEED_AGENT_JAR: &str = "reference/oraclekit/build/seed-agent.jar";

/// The seed agent jar: `$ORACLEKIT_SEED_AGENT` if set, else [`SEED_AGENT_JAR`] under `repo`,
/// built with [`SEED_AGENT_SCRIPT`] if missing.
pub fn seed_agent(repo: &Path) -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("ORACLEKIT_SEED_AGENT").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let jar = repo.join(SEED_AGENT_JAR);
    if !jar.is_file() {
        let st = Command::new(repo.join(SEED_AGENT_SCRIPT))
            .stdout(Stdio::null())
            .status()
            .map_err(|e| format!("{SEED_AGENT_SCRIPT}: {e}"))?;
        if !st.success() || !jar.is_file() {
            return Err(format!("{SEED_AGENT_SCRIPT} failed"));
        }
    }
    Ok(jar)
}

/// The JVM option that loads an agent: `-javaagent:<jar>=<args>` (`-javaagent:<jar>` without
/// arguments).
pub fn agent_option(jar: &Path, args: &str) -> String {
    if args.is_empty() {
        format!("-javaagent:{}", jar.display())
    } else {
        format!("-javaagent:{}={args}", jar.display())
    }
}

/// Runs `script` (a runner's `build.sh`) unless every file of `outputs` exists. The error
/// names the script relative to its runner (`<dir>/build.sh failed`).
pub fn ensure_built(script: &Path, outputs: &[&Path]) -> Result<(), String> {
    if outputs.iter().all(|p| p.is_file()) {
        return Ok(());
    }
    let name = script
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let st = Command::new(script)
        .status()
        .map_err(|e| format!("{name}: {e}"))?;
    if !st.success() {
        return Err(format!("{} failed", script.display()));
    }
    Ok(())
}

/// The stack trace printed for a failure whose message line is `head` (the exception line, as
/// `Throwable.toString` prints it): that line, every `Caused by:` line and the first three frames
/// below each. Empty if `head` is not a line of `stderr`.
pub fn stack_excerpt(stderr: &str, head: &str) -> String {
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

/// The JVM could not create a thread (process or address-space limits) or ran out of memory.
/// SimuLizar runs a Java thread per simulated process: such a run may end early and still
/// report success, so a batch whose standard error shows this is not to be trusted.
pub fn jvm_trouble(stderr: &str) -> bool {
    stderr.contains("pthread_create failed")
        || stderr.contains("unable to create native thread")
        || stderr.contains("OutOfMemoryError")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt() {
        let err = "noise\njava.lang.RuntimeException: boom\n\tat a.b(C.java:1)\n\tat a.c(C.java:2)\n\tat a.d(C.java:3)\n\tat a.e(C.java:4)\nCaused by: java.lang.NullPointerException: x\n\tat q.r(S.java:9)\n\t... 5 more\n[refsim] next\n";
        let e = stack_excerpt(err, "java.lang.RuntimeException: boom");
        assert_eq!(
            e,
            "java.lang.RuntimeException: boom\n    at a.b(C.java:1)\n    at a.c(C.java:2)\n    at a.d(C.java:3)\nCaused by: java.lang.NullPointerException: x\n    at q.r(S.java:9)"
        );
        assert_eq!(stack_excerpt(err, "other"), "");
    }

    #[test]
    fn agent_options() {
        let j = Path::new("/x/seed-agent.jar");
        assert_eq!(
            agent_option(j, ORDERED_QVTO),
            "-javaagent:/x/seed-agent.jar=ordered,qvto"
        );
        assert_eq!(agent_option(j, ""), "-javaagent:/x/seed-agent.jar");
    }

    #[test]
    fn trouble() {
        assert!(jvm_trouble(
            "[0.1s][warning][os,thread] Failed to start thread - pthread_create failed (EAGAIN)"
        ));
        assert!(jvm_trouble("java.lang.OutOfMemoryError: Java heap space"));
        assert!(!jvm_trouble("[refsim] bootstrap 2360 ms"));
    }
}
