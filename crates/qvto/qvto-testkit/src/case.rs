//! Case files: the job lists of the conformance and differential harness.
//!
//! A case file is a sequence of blocks; the Java runner (`reference/runner`, `Job.java`) reads
//! the same format. Format: `crates/qvto-testkit/README.md`, "Case files".

use std::path::{Path, PathBuf};

/// One check of a case: an assertion of the Eclipse driver the case comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// Severity and code of the execution diagnostic (`Diagnostic.OK` = 0, `WARNING` = 2,
    /// `ERROR` = 4, `CANCEL` = 8).
    Severity { severity: i32, code: i32 },
    /// The out/inout extents equal the expected models under the suite's comparer.
    Model,
    /// The log equals the file (`nonempty`: both must be non-empty).
    Log { file: String, nonempty: bool },
    /// The printed QVT stack trace of the terminating exception equals the file.
    Stack { file: String },
    /// Java class of the terminating QVT exception.
    Exception { class: String },
    /// No terminating exception.
    NoException,
    /// Number of trace records after the run.
    TraceRecords(usize),
    /// The first `count - 1` stack frames have this operation, module, unit and one of the lines.
    StackFrames { count: usize, operation: String, module: String, unit: String, lines: Vec<i32> },
    /// Compile only: error and warning counts over the unit and its imports (`warnings` -1: any;
    /// `cst_only`: only `.qvto` units count).
    Problems { errors: i64, warnings: i64, cst_only: bool },
    /// Compile only: the transformation fails to load.
    LoadError,
}

impl Check {
    pub fn name(&self) -> &'static str {
        match self {
            Check::Severity { .. } => "severity",
            Check::Model => "model",
            Check::Log { nonempty: false, .. } => "log",
            Check::Log { nonempty: true, .. } => "log-nonempty-equal",
            Check::Stack { .. } => "stack",
            Check::Exception { .. } => "exception",
            Check::NoException => "no-exception",
            Check::TraceRecords(_) => "trace-records",
            Check::StackFrames { .. } => "stack-frames",
            Check::Problems { .. } => "problems",
            Check::LoadError => "load-error",
        }
    }

    /// The check's line in the case file (without `check `), as the Java runner prints it in
    /// `result.txt`.
    pub fn spec(&self) -> String {
        match self {
            Check::Severity { severity, code } => format!("severity {severity} {code}"),
            Check::Model => "model".into(),
            Check::Log { file, nonempty } => {
                format!("{} {file}", if *nonempty { "log-nonempty-equal" } else { "log" })
            }
            Check::Stack { file } => format!("stack {file}"),
            Check::Exception { class } => format!("exception {class}"),
            Check::NoException => "no-exception".into(),
            Check::TraceRecords(n) => format!("trace-records {n}"),
            Check::StackFrames { count, operation, module, unit, lines } => {
                let l: Vec<String> = lines.iter().map(|x| x.to_string()).collect();
                format!("stack-frames {count} {operation} {module} {unit} {}", l.join("|"))
            }
            Check::Problems { errors, warnings, cst_only } => {
                format!("problems {errors} {warnings}{}", if *cst_only { " cst" } else { "" })
            }
            Check::LoadError => "load-error".into(),
        }
    }

    /// Compile-only checks: the case is not executed.
    pub fn compile_only(&self) -> bool {
        matches!(self, Check::Problems { .. } | Check::LoadError)
    }
}

/// `BlackboxRegistry.registerModule(class[, unit, module[, packageURIs]])`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blackbox {
    pub class: String,
    pub unit: Option<String>,
    pub module: Option<String>,
    pub ns_uris: Vec<String>,
}

/// A case of a case file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Case {
    pub id: String,
    /// The Eclipse test class (`driver`).
    pub driver: String,
    /// The inventory's expected kind (`model`, `assert`, `none`, `log`, `stack`, `trace`, `error`,
    /// `compile_ok`, `api`, `perf`, or a corpus kind).
    pub kind: String,
    pub features: Vec<String>,
    /// Directory of the case's files, relative to the base directory.
    pub dir: Option<String>,
    pub qvto: Option<String>,
    /// Models for the in/inout parameters, in signature order (positional).
    pub inputs: Vec<String>,
    /// Expected models of the out/inout parameters, in signature order.
    pub expected: Vec<String>,
    /// `.ecore` files registered in the resource set (`includeMetamodelFile`).
    pub mmfiles: Vec<String>,
    /// nsURIs of globally registered packages put into the resource set (`getUsedPackages`).
    pub packages: Vec<String>,
    /// Configuration properties; `None` is a null value.
    pub config: Vec<(String, Option<String>)>,
    pub blackboxes: Vec<Blackbox>,
    /// Classes registered with `BlackboxRegistry.registerModules`.
    pub blackbox_modules: Vec<String>,
    /// All blackboxes of the test bundle's `plugin.xml` (drivers that run under OSGi).
    pub bundle_blackboxes: bool,
    /// `QVTEvaluationOptions.FLAG_READONLY_GUARD_ENABLED`.
    pub readonly_guard: bool,
    /// `QVTEvaluationOptions.EVALUATION_MAX_STACK_DEPTH`.
    pub max_stack_depth: Option<u32>,
    /// Trace file for incremental update (`INCREMENTAL_UPDATE_TRACE`).
    pub trace_in: Option<String>,
    /// Compiled without source line numbers.
    pub no_line_numbers: bool,
    /// Imports resolve relative to the transformation's folder, not the bundle's source
    /// containers.
    pub imports_from_folder: bool,
    /// URI prefix maps (`from`, `to` directory relative to the base).
    pub urimaps: Vec<(String, String)>,
    pub checks: Vec<Check>,
    /// Not runnable outside Eclipse: (category, reason).
    pub unrunnable: Option<(String, String)>,
    /// Origin of the case (source file of the driver).
    pub source: Option<String>,
    /// Remarks (`note`, `emulates`).
    pub notes: Vec<String>,
}

impl Case {
    /// A path of the case relative to the base directory (`dir` applied).
    pub fn rel(&self, p: &str) -> String {
        match &self.dir {
            Some(d) if !p.starts_with('/') => format!("{d}/{p}"),
            _ => p.to_string(),
        }
    }

    /// A path of the case under the base directory.
    pub fn path(&self, base: &Path, p: &str) -> PathBuf {
        base.join(self.rel(p))
    }

    pub fn runnable(&self) -> bool {
        self.unrunnable.is_none()
    }

    pub fn compile_only(&self) -> bool {
        self.checks.iter().any(Check::compile_only)
    }

    /// The case's group for reports: the driver class without its package.
    pub fn group(&self) -> &str {
        self.driver.rsplit('.').next().unwrap_or(&self.driver)
    }
}

/// A parse error of a case file.
#[derive(Debug)]
pub struct CaseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for CaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for CaseError {}

/// Reads a case file.
pub fn read_cases(path: &Path) -> Result<Vec<Case>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(parse_cases(&text)?)
}

/// Parses the text of a case file.
pub fn parse_cases(text: &str) -> Result<Vec<Case>, CaseError> {
    let mut out = Vec::new();
    let mut cur: Option<Case> = None;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let err = |m: String| CaseError { line: n, message: m };
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        if key == "case" {
            if cur.is_some() {
                return Err(err("missing end".into()));
            }
            cur = Some(Case { id: rest.to_string(), ..Case::default() });
            continue;
        }
        let Some(c) = cur.as_mut() else {
            continue;
        };
        match key {
            "end" => out.push(cur.take().unwrap()),
            "driver" => c.driver = rest.into(),
            "kind" => c.kind = rest.into(),
            "features" => c.features = rest.split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
            "dir" => c.dir = Some(rest.into()),
            "qvto" => c.qvto = Some(rest.into()),
            "in" => c.inputs.push(rest.into()),
            "expected" => c.expected.push(rest.into()),
            "mmfile" => c.mmfiles.push(rest.into()),
            "package" => c.packages.push(rest.into()),
            "config" => {
                let (k, v) = rest.split_once('=').ok_or_else(|| err(format!("config without '=': {rest}")))?;
                c.config.push((k.into(), Some(v.into())));
            }
            "config-null" => c.config.push((rest.into(), None)),
            "blackbox" => {
                let p: Vec<&str> = rest.split(' ').collect();
                c.blackboxes.push(Blackbox {
                    class: p[0].into(),
                    unit: p.get(1).map(|s| s.to_string()),
                    module: p.get(2).map(|s| s.to_string()),
                    ns_uris: p.get(3).map(|s| s.split(',').map(String::from).collect()).unwrap_or_default(),
                });
            }
            "blackbox-modules" => c.blackbox_modules.push(rest.into()),
            "bundle-blackboxes" => c.bundle_blackboxes = true,
            "session" => {
                let p: Vec<&str> = rest.split(' ').collect();
                match p[0] {
                    "readonly-guard" => c.readonly_guard = true,
                    "max-stack-depth" => {
                        c.max_stack_depth =
                            Some(p.get(1).and_then(|s| s.parse().ok()).ok_or_else(|| err("bad max-stack-depth".into()))?)
                    }
                    o => return Err(err(format!("unknown session option {o}"))),
                }
            }
            "trace-in" => c.trace_in = Some(rest.into()),
            "no-line-numbers" => c.no_line_numbers = true,
            "imports" => c.imports_from_folder = rest == "folder",
            "urimap" => {
                let (a, b) = rest.split_once(' ').ok_or_else(|| err("urimap needs two values".into()))?;
                c.urimaps.push((a.into(), b.into()));
            }
            "check" => c.checks.push(parse_check(rest).map_err(err)?),
            "unrunnable" => {
                let (cat, why) = rest.split_once(' ').unwrap_or((rest, ""));
                c.unrunnable = Some((cat.into(), why.into()));
            }
            "source" => c.source = Some(rest.into()),
            "note" | "emulates" => c.notes.push(format!("{key} {rest}")),
            other => return Err(err(format!("unknown key {other}"))),
        }
    }
    if let Some(c) = cur {
        return Err(CaseError { line: text.lines().count(), message: format!("missing end after case {}", c.id) });
    }
    Ok(out)
}

fn parse_check(s: &str) -> Result<Check, String> {
    let p: Vec<&str> = s.split(' ').collect();
    let int = |i: usize| -> Result<i64, String> {
        p.get(i).and_then(|x| x.parse::<i64>().ok()).ok_or_else(|| format!("check {s}: bad number"))
    };
    let arg = |i: usize| -> Result<String, String> {
        p.get(i).map(|x| x.to_string()).ok_or_else(|| format!("check {s}: missing argument"))
    };
    Ok(match p[0] {
        "severity" => Check::Severity { severity: int(1)? as i32, code: int(2)? as i32 },
        "model" => Check::Model,
        "log" => Check::Log { file: arg(1)?, nonempty: false },
        "log-nonempty-equal" => Check::Log { file: arg(1)?, nonempty: true },
        "stack" => Check::Stack { file: arg(1)? },
        "exception" => Check::Exception { class: arg(1)? },
        "no-exception" => Check::NoException,
        "trace-records" => Check::TraceRecords(int(1)? as usize),
        "stack-frames" => Check::StackFrames {
            count: int(1)? as usize,
            operation: arg(2)?,
            module: arg(3)?,
            unit: arg(4)?,
            lines: arg(5)?.split('|').map(|x| x.parse().map_err(|_| format!("check {s}: bad line"))).collect::<Result<_, _>>()?,
        },
        "problems" => Check::Problems { errors: int(1)?, warnings: int(2)?, cst_only: p.get(3) == Some(&"cst") },
        "load-error" => Check::LoadError,
        o => return Err(format!("unknown check {o}")),
    })
}

/// Writes cases in the case-file format (the inverse of [`parse_cases`] for the fields it
/// knows; used for filtered job lists and generated corpus cases).
pub fn write_case(c: &Case, out: &mut String) {
    use std::fmt::Write;
    let _ = writeln!(out, "case {}", c.id);
    let mut kv = |k: &str, v: &str| {
        let _ = writeln!(out, "{k} {v}");
    };
    if !c.driver.is_empty() {
        kv("driver", &c.driver);
    }
    if !c.kind.is_empty() {
        kv("kind", &c.kind);
    }
    if !c.features.is_empty() {
        kv("features", &c.features.join(","));
    }
    if let Some(d) = &c.dir {
        kv("dir", d);
    }
    if let Some(q) = &c.qvto {
        kv("qvto", q);
    }
    for x in &c.inputs {
        kv("in", x);
    }
    for x in &c.expected {
        kv("expected", x);
    }
    for x in &c.mmfiles {
        kv("mmfile", x);
    }
    for x in &c.packages {
        kv("package", x);
    }
    for (k, v) in &c.config {
        match v {
            Some(v) => kv("config", &format!("{k}={v}")),
            None => kv("config-null", k),
        }
    }
    for b in &c.blackboxes {
        let mut s = b.class.clone();
        for x in [&b.unit, &b.module].into_iter().flatten() {
            s.push(' ');
            s.push_str(x);
        }
        if !b.ns_uris.is_empty() {
            s.push(' ');
            s.push_str(&b.ns_uris.join(","));
        }
        kv("blackbox", &s);
    }
    for x in &c.blackbox_modules {
        kv("blackbox-modules", x);
    }
    if c.bundle_blackboxes {
        kv("bundle-blackboxes", "");
    }
    if c.readonly_guard {
        kv("session", "readonly-guard");
    }
    if let Some(n) = c.max_stack_depth {
        kv("session", &format!("max-stack-depth {n}"));
    }
    if let Some(t) = &c.trace_in {
        kv("trace-in", t);
    }
    if c.no_line_numbers {
        kv("no-line-numbers", "");
    }
    if c.imports_from_folder {
        kv("imports", "folder");
    }
    for (a, b) in &c.urimaps {
        kv("urimap", &format!("{a} {b}"));
    }
    if let Some(s) = &c.source {
        kv("source", s);
    }
    for n in &c.notes {
        let (k, v) = n.split_once(' ').unwrap_or((n, ""));
        kv(k, v);
    }
    for ch in &c.checks {
        kv("check", &ch.spec());
    }
    if let Some((cat, why)) = &c.unrunnable {
        kv("unrunnable", &format!("{cat} {why}"));
    }
    let _ = writeln!(out, "end");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "\
case a
driver transform.TestQvtInterpreter
kind model
dir some/dir
qvto a.qvto
in in.ecore
expected expected.ecore
config k=v=w
config-null n
blackbox x.Y org.bar.Foo Lib http://a,http://b
session max-stack-depth 10
check severity 0 0
check model
check stack-frames 10 op mod unit.qvto 5|6
end
case b
unrunnable api asserts on Java objects
end
";
        let cases = parse_cases(text).unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].config, vec![("k".into(), Some("v=w".into())), ("n".into(), None)]);
        assert_eq!(cases[0].rel("in.ecore"), "some/dir/in.ecore");
        assert!(!cases[1].runnable());
        let mut out = String::new();
        for c in &cases {
            write_case(c, &mut out);
        }
        assert_eq!(parse_cases(&out).unwrap(), cases);
    }
}
