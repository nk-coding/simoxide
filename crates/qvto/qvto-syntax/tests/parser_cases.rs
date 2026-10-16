//! The 155 `TestQvtParser` cases of the Eclipse suite (`parserTestData/sources/<dir>`), read from
//! `TestQvtParser.java`. Eclipse's expected counts are for the whole compiler; this test checks
//! the syntax part:
//!
//! - a case whose files have no syntax problems in Eclipse's parser has none in ours (the
//!   oracle agreement itself is `corpus.rs`);
//! - in the cases with inline annotations (`/*<error>*/…/*</error>*/`, `/*<warn>*/…`), every
//!   problem we report matches an annotation exactly (offset, length, severity), as
//!   `ProblemSourceAnnotationHelper.assertMarchingProblems` requires of Eclipse's problems.
//!
//! It prints how many expected errors are syntax errors and how many annotations the syntax
//! layer already covers; the rest belong to semantic analysis (Q2).

mod common;

use common::*;
use qvto_syntax::Severity;

struct Case {
    dir: String,
    errors: i64,
    warnings: i64,
    annotated: bool,
}

fn cases(java: &str) -> Vec<Case> {
    let mut out = Vec::new();
    for line in java.lines() {
        let l = line.trim();
        if l.starts_with("//") {
            continue;
        }
        let (annotated, rest) = if let Some(r) = l.strip_prefix("TestData.createSourceChecked(\"") {
            (true, r)
        } else if let Some(r) = l.strip_prefix("new TestData(\"") {
            (false, r)
        } else {
            continue;
        };
        let Some((dir, nums)) = rest.split_once('"') else {
            continue;
        };
        let nums: Vec<i64> = nums
            .split(')')
            .next()
            .unwrap()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        out.push(Case {
            dir: dir.to_string(),
            errors: nums[0],
            warnings: nums.get(1).copied().unwrap_or(-1),
            annotated,
        });
    }
    out
}

/// Annotated regions `(offset, length, is_error)` in UTF-16 units.
fn annotations(src: &str) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    for (tag, err) in [("error", true), ("warn", false)] {
        let open = format!("/*<{tag}>*/");
        let close = format!("/*</{tag}>*/");
        let mut from = 0;
        while let Some(i) = src[from..].find(&open) {
            let start = from + i + open.len();
            let Some(j) = src[start..].find(&close) else {
                break;
            };
            let end = start + j;
            let o = qvto_syntax::span::byte_to_utf16(src, start);
            out.push((o, qvto_syntax::span::byte_to_utf16(src, end) - o, err));
            from = end + close.len();
        }
    }
    out
}

#[test]
fn parser_cases() {
    let roots = roots();
    let Some(eclipse) = roots.get("eclipse") else {
        eprintln!("skipped: reference/eclipse-qvto not present");
        return;
    };
    let bundle = eclipse.join("tests/org.eclipse.m2m.tests.qvt.oml");
    let java = read(&bundle.join("src/org/eclipse/m2m/tests/qvt/oml/TestQvtParser.java"));
    let cases = cases(&java);
    assert_eq!(cases.len(), 155, "TestQvtParser.java data list changed");
    let (mut clean_expected, mut clean_ok, mut error_cases, mut syntax_error_cases) = (0, 0, 0, 0);
    let (mut annotated_cases, mut annotations_total, mut annotations_hit, mut unmatched) =
        (0, 0, 0, Vec::new());
    for c in &cases {
        let dir = bundle.join("parserTestData/sources").join(&c.dir);
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "qvto"))
            .collect();
        files.sort();
        let mut syntax_errors = 0;
        for f in &files {
            let src = read(f);
            let parsed = qvto_syntax::parse(&src);
            syntax_errors += parsed.errors().count();
            if c.annotated {
                let ann = annotations(&src);
                annotations_total += ann.len();
                for d in &parsed.diagnostics {
                    let o = qvto_syntax::span::byte_to_utf16(&src, d.span.start as usize);
                    let l = qvto_syntax::span::byte_to_utf16(&src, d.span.end as usize) - o;
                    let hit = ann.iter().any(|&(ao, al, e)| {
                        ao == o && al == l && e == (d.severity == Severity::Error)
                    });
                    if hit {
                        annotations_hit += 1;
                    } else {
                        unmatched.push(format!("{}: {d}", f.display()));
                    }
                }
            }
        }
        if c.annotated {
            annotated_cases += 1;
        }
        if c.errors == 0 {
            clean_expected += 1;
            if syntax_errors == 0 {
                clean_ok += 1;
            }
        } else {
            error_cases += 1;
            if syntax_errors > 0 {
                syntax_error_cases += 1;
            }
        }
        let _ = c.warnings;
    }
    eprintln!(
        "parser cases: {} cases; {clean_expected} expect no errors, {clean_ok} of them without syntax errors; \
         {error_cases} expect errors, {syntax_error_cases} of them have syntax errors (the rest are semantic); \
         {annotated_cases} annotated cases with {annotations_total} annotations, {annotations_hit} matched exactly by syntax diagnostics, {} syntax diagnostics without annotation",
        cases.len(),
        unmatched.len()
    );
    assert_eq!(clean_ok, clean_expected);
    assert!(unmatched.is_empty(), "{}", unmatched.join("\n"));
}
