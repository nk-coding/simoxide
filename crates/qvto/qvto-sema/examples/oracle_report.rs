//! Compares `qvto_sema::compile` with Eclipse QVTo's full-compile oracle over
//! `tests/data/compile.cases` (harness: `tests/oracle_harness/mod.rs`).
//!
//! ```sh
//! CARGO_BUILD_JOBS=4 nice -n 19 cargo run --release -p qvto-sema --example oracle_report -- \
//!     [--filter substr] [--out path.tsv] [--top N] [--timeout secs]
//! ```
//!
//! Prints match statistics, the most frequent missing/extra messages (quoted names normalised)
//! and writes every mismatch to a TSV (default `target/q2-oracle-report.tsv` under the
//! workspace): `key kind(missing|extra) unit sev line offset len msg`.

#[path = "../tests/oracle_harness/mod.rs"]
mod oracle_harness;

use oracle_harness::{CaseRun, Harness, Prob, RunStatus, compare, group, problem_check};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::time::Duration;

/// A message with its quoted parts (`'...'`, `"..."`) replaced by `'_'`.
fn normalise(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut chars = msg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' || c == '"' {
            let mut inner = String::new();
            let mut closed = false;
            for d in chars.by_ref() {
                if d == c {
                    closed = true;
                    break;
                }
                inner.push(d);
            }
            if closed {
                out.push(c);
                out.push('_');
                out.push(c);
            } else {
                out.push(c);
                out.push_str(&inner);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Default, Clone, Copy)]
struct Tally {
    oracle: usize,
    ours: usize,
    matched: usize,
    missing: usize,
    extra: usize,
}

impl Tally {
    fn add(&mut self, o: usize, u: usize, m: usize, mi: usize, e: usize) {
        self.oracle += o;
        self.ours += u;
        self.matched += m;
        self.missing += mi;
        self.extra += e;
    }
}

fn pct(a: usize, b: usize) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.1}%", 100.0 * a as f64 / b as f64)
    }
}

fn main() {
    let mut filter: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut top = 40usize;
    let mut timeout = 120u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--filter" => filter = args.next(),
            "--out" => out_path = args.next(),
            "--top" => top = args.next().and_then(|s| s.parse().ok()).unwrap_or(top),
            "--timeout" => timeout = args.next().and_then(|s| s.parse().ok()).unwrap_or(timeout),
            "-h" | "--help" => {
                eprintln!("oracle_report [--filter substr] [--out path.tsv] [--top N] [--timeout secs]");
                return;
            }
            o => {
                eprintln!("unknown argument {o}");
                std::process::exit(2);
            }
        }
    }
    let mut h = match Harness::new() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("harness setup failed: {e}");
            std::process::exit(1);
        }
    };
    h.timeout = Duration::from_secs(timeout);
    let out_path = out_path.unwrap_or_else(|| h.base.join("target/q2-oracle-report.tsv").to_string_lossy().into_owned());
    println!("setup: {:.2}s (registry, resolvers)", h.setup_time.as_secs_f64());
    for n in &h.notes {
        println!("  setup note: {n}");
    }

    let cases: Vec<_> = h
        .cases
        .iter()
        .filter(|c| {
            filter.as_deref().is_none_or(|f| {
                c.id.contains(f) || c.notes.iter().any(|n| n.starts_with("note key ") && n.contains(f))
            })
        })
        .cloned()
        .collect();

    let mut runs: Vec<CaseRun> = Vec::with_capacity(cases.len());
    for (i, c) in cases.iter().enumerate() {
        let r = h.run_case(c);
        if !matches!(r.status, RunStatus::Ok) {
            eprintln!("[{}/{}] {}: {:?}", i + 1, cases.len(), r.key, r.status);
        }
        runs.push(r);
    }

    // per key
    let mut tsv = String::from("key\tkind\tunit\tsev\tline\toffset\tlen\tmsg\n");
    let mut full_match = 0;
    let (mut ecl_ok, mut ecl_ok_we_ok, mut ecl_ok_we_exact) = (0, 0, 0);
    let (mut ecl_err, mut ecl_err_exact, mut ecl_err_we_err) = (0, 0, 0);
    let (mut ecl_warn_only, mut ecl_warn_only_exact) = (0, 0);
    let mut has_err_agree = 0;
    let mut by_sev: BTreeMap<char, Tally> = BTreeMap::new();
    let mut by_group: BTreeMap<String, Tally> = BTreeMap::new();
    let mut keys_by_group: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_env: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut total = Tally::default();
    let mut missing_msgs: HashMap<String, usize> = HashMap::new();
    let mut extra_msgs: HashMap<String, usize> = HashMap::new();
    let mut same_msg_other_pos = 0;
    let mut panics: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut timeouts = Vec::new();
    let mut setup_failed = Vec::new();
    let mut no_oracle = Vec::new();
    // check problems: (cases, ours pass, oracle pass) for TestQvtParser counts / default `0 -1`
    let mut chk_parser = (0, 0, 0);
    let mut chk_default = (0, 0, 0);
    let mut total_time = Duration::ZERO;
    let mut max_time = (Duration::ZERO, String::new());
    let mut slow: Vec<(Duration, String)> = Vec::new();

    for (case, run) in cases.iter().zip(&runs) {
        match &run.status {
            RunStatus::Ok => {}
            RunStatus::Panic(p) => {
                panics.entry(p.clone()).or_default().push(run.key.clone());
                continue;
            }
            RunStatus::Timeout => {
                timeouts.push(run.key.clone());
                continue;
            }
            RunStatus::Setup(e) => {
                setup_failed.push(format!("{}: {e}", run.key));
                continue;
            }
        }
        total_time += run.elapsed;
        if run.elapsed > max_time.0 {
            max_time = (run.elapsed, run.key.clone());
        }
        slow.push((run.elapsed, run.key.clone()));
        let Some(oracle) = h.oracle_for(&run.key) else {
            no_oracle.push(run.key.clone());
            continue;
        };
        let (matched, missing, extra) = compare(&oracle.problems, &run.problems);
        let exact = missing.is_empty() && extra.is_empty();
        let g = group(&run.key).to_string();
        let kg = keys_by_group.entry(g.clone()).or_default();
        kg.0 += 1;
        let env = oracle_harness::case_env(case);
        let ke = by_env.entry(env).or_default();
        ke.0 += 1;
        if exact {
            full_match += 1;
            kg.1 += 1;
            ke.1 += 1;
        }
        let o_err = oracle.problems.iter().any(|p| p.sev == 'E');
        let u_err = run.problems.iter().any(|p| p.sev == 'E');
        if o_err == u_err {
            has_err_agree += 1;
        }
        if oracle.problems.is_empty() {
            ecl_ok += 1;
            if run.problems.is_empty() {
                ecl_ok_we_ok += 1;
            }
            if !u_err {
                ecl_ok_we_exact += 1;
            }
        } else if o_err {
            ecl_err += 1;
            if exact {
                ecl_err_exact += 1;
            }
            if u_err {
                ecl_err_we_err += 1;
            }
        } else {
            ecl_warn_only += 1;
            if exact {
                ecl_warn_only_exact += 1;
            }
        }
        for sev in ['E', 'W'] {
            let c = |v: &[Prob]| v.iter().filter(|p| p.sev == sev).count();
            by_sev.entry(sev).or_default().add(
                c(&oracle.problems),
                c(&run.problems),
                c(&matched),
                c(&missing),
                c(&extra),
            );
        }
        let t = (oracle.problems.len(), run.problems.len(), matched.len(), missing.len(), extra.len());
        by_group.entry(g).or_default().add(t.0, t.1, t.2, t.3, t.4);
        total.add(t.0, t.1, t.2, t.3, t.4);
        // same message (and unit, severity) at another position
        let mut pool: HashMap<(&str, char, &str), usize> = HashMap::new();
        for p in &extra {
            *pool.entry((&p.unit, p.sev, &p.msg)).or_default() += 1;
        }
        for p in &missing {
            if let Some(n) = pool.get_mut(&(p.unit.as_str(), p.sev, p.msg.as_str()))
                && *n > 0
            {
                *n -= 1;
                same_msg_other_pos += 1;
            }
        }
        for (kind, list, tally) in [("missing", &missing, &mut missing_msgs), ("extra", &extra, &mut extra_msgs)] {
            for p in list.iter() {
                *tally.entry(format!("{} {}", p.sev, normalise(&p.msg))).or_default() += 1;
                let _ = writeln!(
                    tsv,
                    "{}\t{kind}\t{}\t{}\t{}\t{}\t{}\t{}",
                    run.key, p.unit, p.sev, p.line, p.offset, p.len, p.msg
                );
            }
        }
        if let Some((e, w, cst)) = problem_check(case) {
            let count = |v: &[Prob]| {
                let keep = |p: &&Prob| !cst || p.unit.ends_with(".qvto");
                let e = v.iter().filter(keep).filter(|p| p.sev == 'E').count() as i64;
                let w = v.iter().filter(keep).filter(|p| p.sev == 'W').count() as i64;
                (e, w)
            };
            let pass = |(ce, cw): (i64, i64)| ce == e && (w == -1 || cw == w);
            let slot = if (e, w) == (0, -1) { &mut chk_default } else { &mut chk_parser };
            slot.0 += 1;
            if pass(count(&run.problems)) {
                slot.1 += 1;
            }
            if pass(count(&oracle.problems)) {
                slot.2 += 1;
            }
        }
    }

    let ran = runs.iter().filter(|r| r.status == RunStatus::Ok).count();
    let compared = ran - no_oracle.len();
    println!();
    println!("== qvto-sema vs Eclipse compile oracle ==");
    println!("keys: {} selected, {ran} compiled, {compared} compared", cases.len());
    println!(
        "  panics {}, timeouts {}, setup failures {}, without oracle entry {}",
        panics.values().map(Vec::len).sum::<usize>(),
        timeouts.len(),
        setup_failed.len(),
        no_oracle.len()
    );
    println!("keys fully matching: {full_match}/{compared} ({})", pct(full_match, compared));
    for (g, (n, m)) in &keys_by_group {
        println!("  {g:<10} {m}/{n} ({})", pct(*m, *n));
    }
    print!("  by env rule:");
    for (e, (n, m)) in &by_env {
        print!(" {e} {m}/{n};");
    }
    println!();
    println!(
        "Eclipse OK (no problems): {ecl_ok} keys; we have no problems on {ecl_ok_we_ok}, no errors on {ecl_ok_we_exact}"
    );
    println!(
        "Eclipse with errors: {ecl_err} keys; exact match {ecl_err_exact}, we report errors on {ecl_err_we_err}"
    );
    println!("Eclipse with warnings only: {ecl_warn_only} keys; exact match {ecl_warn_only_exact}");
    println!("has-errors agreement: {has_err_agree}/{compared}");
    println!();
    println!(
        "{:<12} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "problems", "eclipse", "ours", "matched", "missing", "extra", "recall"
    );
    let row = |name: &str, t: &Tally| {
        println!(
            "{name:<12} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            t.oracle,
            t.ours,
            t.matched,
            t.missing,
            t.extra,
            pct(t.matched, t.oracle)
        );
    };
    row("total", &total);
    for (s, t) in &by_sev {
        row(if *s == 'E' { "errors" } else { "warnings" }, t);
    }
    for (g, t) in &by_group {
        row(g, t);
    }
    println!("missing+extra pairs with the same unit/severity/message at another position: {same_msg_other_pos}");
    println!();
    println!(
        "check problems (TestQvtParser counts): {} cases, ours pass {}, Eclipse oracle passes {}",
        chk_parser.0, chk_parser.1, chk_parser.2
    );
    println!(
        "check problems 0 -1 (no errors expected): {} cases, ours pass {}, Eclipse oracle passes {}",
        chk_default.0, chk_default.1, chk_default.2
    );
    println!();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "analysis time (compile calls only): total {:.1} ms, mean {:.3} ms, max {:.1} ms ({})",
        ms(total_time),
        if ran > 0 { ms(total_time) / ran as f64 } else { 0.0 },
        ms(max_time.0),
        max_time.1
    );
    // TestQvtParser's inline annotations (ProblemSourceAnnotationHelper.assertMatchingProblems):
    // every problem of an annotated case hits an annotation exactly and vice versa
    {
        let bundle = h.base.join("reference/eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml");
        let java = std::fs::read_to_string(bundle.join("src/org/eclipse/m2m/tests/qvt/oml/TestQvtParser.java")).unwrap_or_default();
        let annotated: Vec<String> = java
            .lines()
            .filter_map(|l| l.trim().strip_prefix("TestData.createSourceChecked(\"").and_then(|r| r.split_once('"')).map(|(d, _)| d.to_string()))
            .collect();
        let (mut cases_n, mut cases_ok, mut ann_total, mut ann_hit, mut probs_total, mut probs_hit) = (0, 0, 0, 0, 0, 0);
        let mut ora = (0, 0);
        for (case, run) in cases.iter().zip(&runs) {
            let Some(dir) = annotated.iter().find(|d| run.key.ends_with(&format!("parserTestData/sources/{d}/{d}.qvto"))) else {
                continue;
            };
            let dirp = bundle.join("parserTestData/sources").join(dir);
            let Ok(rd) = std::fs::read_dir(&dirp) else {
                continue;
            };
            let mut ann: Vec<(String, i64, i64, char)> = Vec::new();
            for f in rd.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|x| x == "qvto") {
                    let src = std::fs::read_to_string(&p).unwrap_or_default();
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    for (tag, sev) in [("error", 'E'), ("warn", 'W')] {
                        let open = format!("/*<{tag}>*/");
                        let close = format!("/*</{tag}>*/");
                        let mut from = 0;
                        while let Some(i) = src[from..].find(&open) {
                            let start = from + i + open.len();
                            let Some(j) = src[start..].find(&close) else { break };
                            let end = start + j;
                            let o = src[..start].encode_utf16().count() as i64;
                            let l = src[start..end].encode_utf16().count() as i64;
                            ann.push((name.clone(), o, l, sev));
                            from = end + close.len();
                        }
                    }
                }
            }
            cases_n += 1;
            let hit = |p: &Prob| ann.iter().any(|(u, o, l, s)| *u == p.unit && *o == p.offset && *l == p.len && *s == p.sev);
            let covered = |a: &(String, i64, i64, char), ps: &[Prob]| ps.iter().any(|p| p.unit == a.0 && p.offset == a.1 && p.len == a.2 && p.sev == a.3);
            let ph = run.problems.iter().filter(|p| hit(p)).count();
            let ah = ann.iter().filter(|a| covered(a, &run.problems)).count();
            probs_total += run.problems.len();
            probs_hit += ph;
            ann_total += ann.len();
            ann_hit += ah;
            if ph == run.problems.len() && ah == ann.len() {
                cases_ok += 1;
            }
            if let Some(o) = h.oracle_for(&run.key) {
                if o.problems.iter().all(|p| hit(p)) && ann.iter().all(|a| covered(a, &o.problems)) {
                    ora.0 += 1;
                }
                ora.1 += 1;
            }
            let _ = case;
        }
        println!(
            "TestQvtParser annotations: {cases_n} annotated cases, {cases_ok} fully matching (Eclipse oracle: {}/{}); {ann_hit}/{ann_total} annotations hit by our problems; {probs_hit}/{probs_total} of our problems on an annotation",
            ora.0, ora.1
        );
    }

    // IR consistency of the error-free compilations
    let mut ir_cases = 0;
    let mut ir_bad = 0;
    let mut ir_kinds: HashMap<String, usize> = HashMap::new();
    for run in &runs {
        if !run.problems.iter().any(|p| p.sev == 'E') && matches!(run.status, RunStatus::Ok) {
            ir_cases += 1;
            if !run.ir_issues.is_empty() {
                ir_bad += 1;
                if std::env::var("IR_ISSUES").is_ok() {
                    for i in run.ir_issues.iter().take(3) {
                        println!("  ir {}: {i}", run.key);
                    }
                }
            }
            for i in &run.ir_issues {
                let k: String = i.split(": ").skip(1).collect::<Vec<_>>().join(": ");
                let k = k.split(" at ").next().unwrap_or("").to_string();
                *ir_kinds.entry(k).or_default() += 1;
            }
        }
    }
    println!("IR checks (irck) on error-free compilations: {ir_cases} cases, {ir_bad} with issues");
    let mut kv: Vec<_> = ir_kinds.into_iter().collect();
    kv.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in kv.iter().take(15) {
        println!("  {n:6}  {k}");
    }
    slow.sort_by(|a, b| b.0.cmp(&a.0));
    for (d, k) in slow.iter().skip(1).take(4) {
        println!("  {:.1} ms {k}", ms(*d));
    }

    let print_top = |title: &str, m: &HashMap<String, usize>| {
        let mut v: Vec<(&String, &usize)> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        println!();
        println!("== top {top} {title} ({} distinct, {} total) ==", v.len(), m.values().sum::<usize>());
        for (msg, n) in v.into_iter().take(top) {
            println!("{n:>6}  {msg}");
        }
    };
    print_top("missing (Eclipse only)", &missing_msgs);
    print_top("extra (ours only)", &extra_msgs);

    if !panics.is_empty() {
        println!();
        println!("== panics ==");
        for (p, keys) in &panics {
            println!("{:>4}x {p}", keys.len());
            for k in keys.iter().take(3) {
                println!("       {k}");
            }
        }
    }
    for t in &timeouts {
        println!("timeout: {t}");
    }
    for s in &setup_failed {
        println!("setup failed: {s}");
    }
    for k in &no_oracle {
        println!("no oracle entry: {k}");
    }
    if let Some(dir) = std::path::Path::new(&out_path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&out_path, tsv) {
        Ok(()) => println!("\nmismatches written to {out_path}"),
        Err(e) => eprintln!("cannot write {out_path}: {e}"),
    }
    // leave even if a timed-out case's thread still runs
    std::process::exit(0);
}
