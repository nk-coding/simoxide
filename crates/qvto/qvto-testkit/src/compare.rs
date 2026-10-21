//! Comparison of an engine's outcome with the case's checks (the Eclipse drivers' assertions:
//! the conformance verdict) and with the Java runner's result (the strict differential verdict).

use crate::case::{Case, Check};
use crate::engine::{CaseEnv, Outcome, Severity, Status};
use crate::fixture::JavaResult;
use qvto_emf::canon;
use qvto_emf::compare::suite_equal;
use qvto_model::ModelStore;

/// Result of one check.
#[derive(Clone, Debug)]
pub struct CheckResult {
    pub check: String,
    pub pass: bool,
    pub detail: String,
}

/// Evaluates the case's checks on an outcome, as the Eclipse driver asserts them.
pub fn check_outcome(env: &CaseEnv, outcome: &Outcome) -> Vec<CheckResult> {
    env.case
        .checks
        .iter()
        .map(|c| {
            let r = check_one(env, c, outcome);
            CheckResult {
                check: c.name().into(),
                pass: r.is_ok(),
                detail: r.err().unwrap_or_default(),
            }
        })
        .collect()
}

fn read(env: &CaseEnv, f: &str) -> Result<String, String> {
    std::fs::read_to_string(env.path(f)).map_err(|e| format!("{f}: {e}"))
}

fn check_one(env: &CaseEnv, c: &Check, o: &Outcome) -> Result<(), String> {
    match c {
        Check::Severity { severity, code } => {
            let got = o.severity.map(|s| s as i32);
            if got == Some(*severity) && o.code == *code {
                Ok(())
            } else {
                Err(format!(
                    "expected severity {severity} code {code}, got {got:?} code {}: {}",
                    o.code, o.message
                ))
            }
        }
        Check::Model => check_models(env, o),
        Check::Log { file, nonempty } => {
            let exp = read(env, file)?;
            if *nonempty && (exp.is_empty() || o.log.is_empty()) {
                return Err("empty log".into());
            }
            text_equal("log", &exp, &o.log)
        }
        Check::Stack { file } => {
            let exp = read(env, file)?;
            text_equal("stack", &exp, o.stack.as_deref().ok_or("no stack trace")?)
        }
        Check::Exception { class } => {
            if o.exception.as_deref() == Some(class.as_str()) {
                Ok(())
            } else {
                Err(format!("expected exception {class}, got {:?}", o.exception))
            }
        }
        Check::NoException => match &o.exception {
            None => Ok(()),
            Some(e) => Err(format!("unexpected exception {e}")),
        },
        Check::TraceRecords(n) => {
            if o.trace_records == Some(*n) {
                Ok(())
            } else {
                Err(format!(
                    "expected {n} trace records, got {:?}",
                    o.trace_records
                ))
            }
        }
        Check::StackFrames {
            count,
            operation,
            module,
            unit,
            lines,
        } => {
            let frames = stack_frames(o.stack.as_deref().ok_or("no stack trace")?);
            if frames.is_empty() {
                return Err("no stack trace".into());
            }
            for f in frames.iter().take(count.saturating_sub(1)) {
                if f.operation != *operation
                    || f.module != *module
                    || f.unit != *unit
                    || !lines.contains(&f.line)
                {
                    return Err(format!("frame {f:?}"));
                }
            }
            Ok(())
        }
        Check::Problems {
            errors,
            warnings,
            cst_only,
        } => {
            if o.load.is_none() {
                return Err("not compiled".into());
            }
            let counted = |err: bool| {
                o.problems
                    .iter()
                    .filter(|p| p.error == err && (!cst_only || p.unit.ends_with(".qvto")))
                    .count() as i64
            };
            let (e, w) = (counted(true), counted(false));
            if e == *errors && (*warnings == -1 || w == *warnings) {
                Ok(())
            } else {
                Err(format!(
                    "expected {errors} errors, {warnings} warnings, got {e}, {w}"
                ))
            }
        }
        Check::LoadError => match o.load {
            Some(Severity::Error) => Ok(()),
            l => Err(format!("the transformation loads ({l:?})")),
        },
    }
}

fn text_equal(what: &str, exp: &str, got: &str) -> Result<(), String> {
    if exp == got {
        return Ok(());
    }
    let i = exp
        .bytes()
        .zip(got.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    Err(format!(
        "{what} differs at byte {i} of {}/{}",
        exp.len(),
        got.len()
    ))
}

/// The out/inout extents against the expected models with the suite's comparer
/// (`ModelTestData.compareWithExpected`).
fn check_models(env: &CaseEnv, o: &Outcome) -> Result<(), String> {
    let exp = &env.case.expected;
    let Some(out) = &o.outputs else {
        return if exp.is_empty() {
            Ok(())
        } else {
            Err("no outputs".into())
        };
    };
    for (i, &r) in out.extents.iter().enumerate() {
        let f = exp
            .get(i)
            .ok_or_else(|| format!("no expected model for output {i}"))?;
        let p = env.path(f);
        let mut rs = env.models.resource_set_for(&p);
        let er = rs.load_file(&p).map_err(|e| format!("{f}: {e}"))?;
        rs.resolve_all();
        let want = rs.graph.roots(er).to_vec();
        let got = out.rs.graph.roots(r).to_vec();
        if want.len() != got.len() {
            return Err(format!(
                "output {i}: {} roots, expected {} ({f})",
                got.len(),
                want.len()
            ));
        }
        if !suite_equal(&rs.graph, &want, &out.rs.graph, &got) {
            return Err(format!("output {i} differs from {f}"));
        }
    }
    Ok(())
}

/// A stack frame of a printed QVT stack trace (`\tat module::[context::]operation(unit:line)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub module: String,
    pub operation: String,
    pub unit: String,
    pub line: i32,
}

pub fn stack_frames(stack: &str) -> Vec<Frame> {
    stack
        .lines()
        .filter_map(|l| {
            let l = l.strip_prefix("\tat ")?;
            let (name, loc) = l.split_once('(')?;
            let loc = loc.strip_suffix(')')?;
            let (unit, line) = loc.rsplit_once(':').unwrap_or((loc, "-1"));
            let segs: Vec<&str> = name.split("::").collect();
            Some(Frame {
                module: segs[0].into(),
                operation: segs[segs.len() - 1].into(),
                unit: unit.into(),
                line: line.parse().unwrap_or(-1),
            })
        })
        .collect()
}

/// One aspect of the differential comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Aspect {
    Equal,
    /// Not comparable: the Java output is not reproducible (identity-hash order, time).
    Unstable,
    Differs(String),
}

/// The engine's outcome against the Java runner's result.
#[derive(Clone, Debug)]
pub struct Differential {
    pub status: Aspect,
    pub dump: Aspect,
    pub log: Aspect,
    pub stack: Aspect,
    pub problems: Aspect,
}

impl Differential {
    /// Strictly equal: every comparable aspect equal (unstable aspects do not count).
    pub fn equal(&self) -> bool {
        [
            &self.status,
            &self.dump,
            &self.log,
            &self.stack,
            &self.problems,
        ]
        .iter()
        .all(|a| !matches!(a, Aspect::Differs(_)))
    }

    pub fn first_difference(&self) -> Option<String> {
        [
            ("status", &self.status),
            ("dump", &self.dump),
            ("log", &self.log),
            ("stack", &self.stack),
            ("problems", &self.problems),
        ]
        .into_iter()
        .find_map(|(n, a)| match a {
            Aspect::Differs(d) => Some(format!("{n}: {d}")),
            _ => None,
        })
    }
}

/// Compares an engine's outcome with the Java runner's result: severity, code and exception;
/// the canonical dump of the outputs (strict: order and reference targets); the log; the stack
/// trace; the compile problems by severity and line.
pub fn differential(case: &Case, o: &Outcome, java: &JavaResult) -> Differential {
    differential_with(case, o, java, false)
}

/// [`differential`]; with `compare_unstable`, aspects the fixture marks unstable (they vary
/// between Java runs, e.g. identity-hash orders) are compared too: the engine replays the
/// recorded run's identity hashes, so it should reproduce that run exactly.
pub fn differential_with(
    case: &Case,
    o: &Outcome,
    java: &JavaResult,
    compare_unstable: bool,
) -> Differential {
    let unstable = if compare_unstable {
        Vec::new()
    } else {
        java.unstable()
    };
    let status = {
        let mut d = Vec::new();
        if o.load.map(Severity::name) != java.get("load") {
            d.push(format!(
                "load {:?} vs {:?}",
                o.load.map(Severity::name),
                java.get("load")
            ));
        }
        if !case.compile_only() {
            if o.severity.map(Severity::name) != java.severity() {
                d.push(format!(
                    "severity {:?} vs {:?}",
                    o.severity.map(Severity::name),
                    java.severity()
                ));
            }
            if Some(o.code) != java.code() {
                d.push(format!("code {} vs {:?}", o.code, java.code()));
            }
            if o.exception.as_deref() != java.exception() {
                d.push(format!(
                    "exception {:?} vs {:?}",
                    o.exception,
                    java.exception()
                ));
            }
        }
        if d.is_empty() {
            Aspect::Equal
        } else {
            Aspect::Differs(d.join("; "))
        }
    };
    let dump = if case.compile_only() {
        Aspect::Equal
    } else {
        let mine = o
            .outputs
            .as_ref()
            .map(|out| canon::dump(&out.rs.graph, &out.dumped()))
            .unwrap_or_default();
        if java.dump_equals(&mine) {
            Aspect::Equal
        } else if unstable.contains(&"dump") {
            Aspect::Unstable
        } else if java.dump_digest().is_some() {
            Aspect::Differs(
                "differs from the digest of the Java dump (third-party models: the fixture keeps \
                 only the digest; the full dump is in the cache of `qvto conform record`)"
                    .into(),
            )
        } else {
            let mine = renumber_extent_resources(&mine);
            let theirs = renumber_extent_resources(java.dump().unwrap_or(""));
            let (mine, theirs) = (mine.as_str(), theirs.as_str());
            match canon::first_difference(theirs, mine) {
                Some((n, a, b)) => Aspect::Differs(format!("line {n}: java {a} | engine {b}")),
                None => Aspect::Equal,
            }
        }
    };
    let log = if o.log == java.log() {
        Aspect::Equal
    } else if unstable.contains(&"log") {
        Aspect::Unstable
    } else {
        Aspect::Differs(
            text_equal("log", java.log(), &o.log)
                .err()
                .unwrap_or_default(),
        )
    };
    let stack = match (o.stack.as_deref(), java.stack()) {
        (a, b) if a == b => Aspect::Equal,
        (a, b) => Aspect::Differs(
            text_equal("stack", b.unwrap_or(""), a.unwrap_or(""))
                .err()
                .unwrap_or_default(),
        ),
    };
    let problems = {
        let key = |e: bool, l: i32| (e, l);
        let mut a: Vec<_> = o.problems.iter().map(|p| key(p.error, p.line)).collect();
        let mut b: Vec<_> = java
            .problems()
            .iter()
            .map(|p| key(p.error, p.line))
            .collect();
        a.sort();
        b.sort();
        if a == b {
            Aspect::Equal
        } else {
            Aspect::Differs(format!(
                "{} problems vs {} (severity, line)",
                a.len(),
                b.len()
            ))
        }
    };
    let status = if o.status != Status::Completed {
        Aspect::Differs(format!("{:?}", o.status))
    } else {
        status
    };
    Differential {
        status,
        dump,
        log,
        stack,
        problems,
    }
}

/// Renumbers QVTo's in-memory extent resources (`extent:/N`) in order of first appearance.
/// Eclipse numbers them with a JVM-wide counter (`ModelParameterExtent.ourExtentId`), so the
/// Java runner's numbers depend on the cases that ran before in the same JVM; the engine counts
/// per run. Only the relative numbering within a dump is compared.
pub fn renumber_extent_resources(dump: &str) -> String {
    const P: &str = "extent:/";
    let mut seen: Vec<&str> = Vec::new();
    let mut out = String::with_capacity(dump.len());
    let mut rest = dump;
    while let Some(i) = rest.find(P) {
        let after = &rest[i + P.len()..];
        let n = after.bytes().take_while(u8::is_ascii_digit).count();
        out.push_str(&rest[..i + P.len()]);
        if n == 0 {
            rest = after;
            continue;
        }
        let num = &after[..n];
        let k = match seen.iter().position(|x| *x == num) {
            Some(k) => k,
            None => {
                seen.push(num);
                seen.len() - 1
            }
        };
        out.push_str(&(k + 1).to_string());
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::renumber_extent_resources as r;

    #[test]
    fn extent_resources_are_renumbered_by_first_appearance() {
        assert_eq!(
            r(r#"{"a":"extent:/7#/0","b":"extent:/3#/1","c":"extent:/7#/2","d":"x.xmi#/0"}"#),
            r#"{"a":"extent:/1#/0","b":"extent:/2#/1","c":"extent:/1#/2","d":"x.xmi#/0"}"#
        );
        assert_eq!(r("extent:/"), "extent:/");
    }
}
