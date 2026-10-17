//! The conformance scoreboard: runs an engine over case files, judges each case by the Eclipse
//! driver's checks and against the Java runner, and summarises by driver, kind and feature.

use crate::case::Case;
use crate::compare::{CheckResult, Differential, check_outcome, differential};
use crate::engine::{CaseEnv, Engine, Status};
use crate::fixture::JavaResult;
use qvto_emf::config::Config;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

/// Verdict for one case.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// The checks pass and the outcome equals the Java runner's (unstable aspects excepted).
    Pass,
    /// The checks pass (the Eclipse suite would pass) but the outcome differs from Java's.
    PassLenient,
    /// A check fails.
    Fail,
    /// The engine does not support something the case needs.
    Unsupported,
    /// The engine cannot run transformations yet.
    NotImplemented,
    /// The engine crashed.
    Crashed,
    /// The Java runner fails the case's checks too (a harness problem, not counted).
    JavaFails,
    /// Not runnable outside Eclipse (by category).
    Unrunnable(String),
}

impl Verdict {
    pub fn label(&self) -> String {
        match self {
            Verdict::Pass => "pass".into(),
            Verdict::PassLenient => "pass-lenient".into(),
            Verdict::Fail => "fail".into(),
            Verdict::Unsupported => "unsupported".into(),
            Verdict::NotImplemented => "not-implemented".into(),
            Verdict::Crashed => "crashed".into(),
            Verdict::JavaFails => "java-fails".into(),
            Verdict::Unrunnable(c) => format!("unrunnable:{c}"),
        }
    }

    /// The case passes as the Eclipse suite judges it.
    pub fn passes(&self) -> bool {
        matches!(self, Verdict::Pass | Verdict::PassLenient)
    }
}

/// The result of one case.
#[derive(Clone, Debug)]
pub struct CaseReport {
    pub id: String,
    pub group: String,
    pub kind: String,
    pub features: Vec<String>,
    pub verdict: Verdict,
    pub checks: Vec<CheckResult>,
    pub differential: Option<Differential>,
    pub detail: String,
}

/// Runs the engine on every case (or the selected ones) and judges the outcomes.
pub fn run_cases(
    engine: &mut dyn Engine,
    cases: &[Case],
    base: &Path,
    models: &Config,
    java: &BTreeMap<String, JavaResult>,
    select: &dyn Fn(&Case) -> bool,
) -> Vec<CaseReport> {
    let mut out = Vec::new();
    for case in cases.iter().filter(|c| select(c)) {
        let mut rep = CaseReport {
            id: case.id.clone(),
            group: case.group().into(),
            kind: case.kind.clone(),
            features: case.features.clone(),
            verdict: Verdict::NotImplemented,
            checks: Vec::new(),
            differential: None,
            detail: String::new(),
        };
        if let Some((cat, why)) = &case.unrunnable {
            rep.verdict = Verdict::Unrunnable(cat.clone());
            rep.detail = why.clone();
            out.push(rep);
            continue;
        }
        let jr = java.get(&case.id);
        if let Some(j) = jr
            && j.verdict() != "pass"
        {
            rep.verdict = Verdict::JavaFails;
            rep.detail = format!("java: {}", j.verdict());
            out.push(rep);
            continue;
        }
        let conf_models;
        let models = match &case.conf {
            Some(c) => match conf_cache(c, base) {
                Ok(m) => {
                    conf_models = m;
                    &conf_models
                }
                Err(e) => {
                    rep.verdict = Verdict::Crashed;
                    rep.detail = format!("conf {c}: {e}");
                    out.push(rep);
                    continue;
                }
            },
            None => models,
        };
        let env = CaseEnv { case, base, models };
        let outcome =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.run(&env))) {
                Ok(o) => o,
                Err(p) => {
                    let msg = p
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    crate::engine::Outcome::with_status(Status::Crashed(format!("panic: {msg}")))
                }
            };
        match &outcome.status {
            Status::NotImplemented(m) => {
                rep.verdict = Verdict::NotImplemented;
                rep.detail = m.clone();
            }
            Status::Unsupported(m) => {
                rep.verdict = Verdict::Unsupported;
                rep.detail = m.clone();
            }
            Status::Crashed(m) => {
                rep.verdict = Verdict::Crashed;
                rep.detail = m.clone();
            }
            Status::Completed => {
                rep.checks = check_outcome(&env, &outcome);
                let failed = rep.checks.iter().find(|c| !c.pass);
                let diff = jr.map(|j| differential(case, &outcome, j));
                rep.verdict = match (failed, &diff) {
                    (Some(f), _) => {
                        rep.detail = format!("{}: {}", f.check, f.detail);
                        Verdict::Fail
                    }
                    (None, Some(d)) if !d.equal() => {
                        rep.detail = d.first_difference().unwrap_or_default();
                        Verdict::PassLenient
                    }
                    _ => Verdict::Pass,
                };
                rep.differential = diff;
            }
        }
        out.push(rep);
    }
    out
}

fn conf_cache(conf: &str, base: &Path) -> Result<Config, String> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<BTreeMap<String, Config>>> = OnceLock::new();
    let mut m = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(c) = m.get(conf) {
        return Ok(c.clone());
    }
    // the Eclipse environment's registrations stay (the Java runner's global registry)
    let suite = Config::read(&base.join(crate::paths::SUITE_CONF), base).map_err(|e| e.to_string())?;
    let c = Config::read(&base.join(conf), base).map_err(|e| e.to_string())?.with_base(&suite);
    m.insert(conf.to_string(), c.clone());
    Ok(c)
}

/// Counts of verdicts in a group.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    pub counts: BTreeMap<String, usize>,
    pub total: usize,
    pub runnable: usize,
    pub passing: usize,
    pub strict: usize,
}

impl Tally {
    fn add(&mut self, v: &Verdict) {
        *self.counts.entry(v.label()).or_default() += 1;
        self.total += 1;
        if !matches!(v, Verdict::Unrunnable(_)) {
            self.runnable += 1;
        }
        if v.passes() {
            self.passing += 1;
        }
        if *v == Verdict::Pass {
            self.strict += 1;
        }
    }
}

/// Tallies of all cases and by driver group, kind and feature tag.
#[derive(Clone, Debug, Default)]
pub struct Tallies {
    pub all: Tally,
    pub by_group: BTreeMap<String, Tally>,
    pub by_kind: BTreeMap<String, Tally>,
    pub by_feature: BTreeMap<String, Tally>,
}

/// Tallies by driver group, by kind and by feature tag.
pub fn tally(reports: &[CaseReport]) -> Tallies {
    let mut all = Tally::default();
    let mut by_group: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_feature: BTreeMap<String, Tally> = BTreeMap::new();
    for r in reports {
        all.add(&r.verdict);
        by_group.entry(r.group.clone()).or_default().add(&r.verdict);
        by_kind.entry(r.kind.clone()).or_default().add(&r.verdict);
        for f in &r.features {
            by_feature.entry(f.clone()).or_default().add(&r.verdict);
        }
    }
    Tallies {
        all,
        by_group,
        by_kind,
        by_feature,
    }
}

fn pct(a: usize, b: usize) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.1} %", 100.0 * a as f64 / b as f64)
    }
}

/// The scoreboard as Markdown: totals, tables by driver, kind and feature, then one line per
/// case that does not pass strictly (with the first failing check or difference).
pub fn markdown(engine: &str, reports: &[CaseReport], verbose: bool) -> String {
    let Tallies {
        all,
        by_group,
        by_kind,
        by_feature,
    } = tally(reports);
    let mut s = String::new();
    let _ = writeln!(s, "# Conformance: {engine}\n");
    let _ = writeln!(
        s,
        "{} cases, {} runnable; {} pass the Eclipse checks ({} of the runnable), {} also equal the Java runner strictly ({}).\n",
        all.total,
        all.runnable,
        all.passing,
        pct(all.passing, all.runnable),
        all.strict,
        pct(all.strict, all.runnable)
    );
    let labels: Vec<String> = all.counts.keys().cloned().collect();
    let table = |s: &mut String, title: &str, m: &BTreeMap<String, Tally>| {
        let _ = writeln!(s, "## By {title}\n");
        let _ = writeln!(
            s,
            "| {title} | cases | runnable | Eclipse checks pass | equal to Java | {} |",
            labels.join(" | ")
        );
        let _ = writeln!(
            s,
            "|---|---:|---:|---:|---:|{}",
            "---:|".repeat(labels.len())
        );
        for (k, t) in m {
            let cols: Vec<String> = labels
                .iter()
                .map(|l| t.counts.get(l).copied().unwrap_or(0).to_string())
                .collect();
            let _ = writeln!(
                s,
                "| {k} | {} | {} | {} | {} | {} |",
                t.total,
                t.runnable,
                t.passing,
                t.strict,
                cols.join(" | ")
            );
        }
        s.push('\n');
    };
    table(&mut s, "driver", &by_group);
    table(&mut s, "kind", &by_kind);
    if verbose {
        table(&mut s, "feature", &by_feature);
    }
    let _ = writeln!(s, "## Cases not passing strictly\n");
    for r in reports {
        if r.verdict == Verdict::Pass
            || (!verbose && matches!(r.verdict, Verdict::Unrunnable(_) | Verdict::NotImplemented))
        {
            continue;
        }
        let _ = writeln!(s, "- `{}` {}: {}", r.id, r.verdict.label(), r.detail);
    }
    s
}

/// One line per case: id, verdict, detail (tab-separated), for diffs between runs.
pub fn tsv(reports: &[CaseReport]) -> String {
    let mut s = String::from("id\tverdict\tdetail\n");
    for r in reports {
        let _ = writeln!(
            s,
            "{}\t{}\t{}",
            r.id,
            r.verdict.label(),
            r.detail.replace(['\t', '\n'], " ")
        );
    }
    s
}
