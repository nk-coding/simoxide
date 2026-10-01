//! Golden-file tests against the Java scheduler oracle (`reference/oracles/sched`): every trace
//! line (state changes, completions, activations, and the bit patterns of all remaining demands
//! after every scheduler call) must match exactly.

mod common;

use std::fs;

use simoxide_sched::PsAlgorithm;

fn check(name: &str) {
    let dir = common::oracle_dir();
    let script = fs::read_to_string(dir.join("scripts").join(format!("{name}.txt"))).unwrap();
    let golden = fs::read_to_string(dir.join("golden").join(format!("{name}.txt"))).unwrap();
    let got = common::run(&common::parse(&script), PsAlgorithm::Exact);
    if got != golden {
        let (gl, el): (Vec<_>, Vec<_>) = (got.lines().collect(), golden.lines().collect());
        let i = gl
            .iter()
            .zip(&el)
            .position(|(a, b)| a != b)
            .unwrap_or(gl.len().min(el.len()));
        let lo = i.saturating_sub(3);
        panic!(
            "{name}: first difference at line {} (got {} lines, expected {})\n--- got\n{}\n--- expected\n{}",
            i + 1,
            gl.len(),
            el.len(),
            gl[lo..(i + 3).min(gl.len())].join("\n"),
            el[lo..(i + 3).min(el.len())].join("\n")
        );
    }
}

#[test]
fn all_golden_files_are_tested() {
    let mut names: Vec<String> = fs::read_dir(common::oracle_dir().join("golden"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter_map(|n| n.strip_suffix(".txt").map(str::to_string))
        .collect();
    names.sort();
    assert!(names.len() >= 30, "golden files missing");
    for n in &names {
        check(n);
    }
}
