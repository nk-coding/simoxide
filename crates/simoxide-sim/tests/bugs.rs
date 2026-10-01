//! Regression tests for bugs found in simoxide-sim by differential testing, and for reference aborts
//! that SimOxide reproduces. A new bug gets a failing test here, `#[ignore = "BUG-n"]` until it is
//! fixed. What each test guards: `docs/correctness/testing.md`, "Regression tests".

use std::path::Path;

use simoxide_testkit::corpus::{self, HarnessOptions, Mode};

/// SplitMix64 for reproducible random bit patterns.
fn next(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `simoxide_sim::javafmt` agrees with the simoxide-testkit formatter (checked against 24 859 Java 21 golden
/// values) on normal doubles.
#[test]
fn javafmt_matches_java_on_normal_doubles() {
    let mut s = 7;
    for _ in 0..200_000 {
        let x = f64::from_bits(next(&mut s));
        if !x.is_finite() || x.is_subnormal() {
            continue;
        }
        assert_eq!(
            simoxide_sim::javafmt::double(x),
            simoxide_testkit::javafmt::to_string(x),
            "bits {:#x}",
            x.to_bits()
        );
    }
}

/// BUG-1: Java's `Double.toString` prints at least two significant digits; when the shortest
/// decimal of a subnormal has one digit, Java picks the closest two-digit decimal
/// (`Double.MIN_VALUE` is `4.9E-324`, simoxide-sim prints `5.0E-324`). Reachable through samples of
/// heavy-tailed distributions (`GammaMoments(0.005, 3.0)` returns 4.9E-324): tape, trace and
/// measurements.csv differ.
#[test]
fn bug1_javafmt_subnormal_two_digit_rule() {
    let mut cases = vec![f64::from_bits(1), f64::from_bits(2), f64::from_bits(20)];
    let mut s = 11;
    for _ in 0..20_000 {
        cases.push(f64::from_bits(next(&mut s) & 0x000F_FFFF_FFFF_FFFF));
    }
    let bad: Vec<String> = cases
        .iter()
        .filter(|x| simoxide_sim::javafmt::double(**x) != simoxide_testkit::javafmt::to_string(**x))
        .take(5)
        .map(|x| {
            format!(
                "{:#x}: simoxide-sim {} java {}",
                x.to_bits(),
                simoxide_sim::javafmt::double(*x),
                simoxide_testkit::javafmt::to_string(*x)
            )
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// BUG-1 end to end: the fuzz-found model whose `GammaMoments` sample is `Double.MIN_VALUE`.
#[test]
fn bug1_subnormal_sample_model() {
    repro("k_bug1_subnormal");
}

/// BUG-2: after a double resume a process is queued twice at an FCFS resource; the reference
/// aborts with a NullPointerException in `SimFCFSResource.scheduleNextEvent` (see
/// `corpus-fuzz/k_bug2_*/REFERENCE-ERROR.txt`). simoxide-sim must report an error too, and its trace
/// must equal the reference's partial trace (`reference-partial/trace.jsonl`) up to the abort
/// (everything but the reference's final `finish` line).
/// Repros: processing FCFS (`k_bug2_fcfs_double_queue`), FCFS linking resource
/// (`k_bug2_link_double_queue`), and a case where the second queueing overwrites the remaining
/// demand of the first job before the abort (`k_bug2_fcfs_overwritten_demand`).
#[test]
fn bug2_fcfs_double_queue_aborts() {
    follows_reference_to_abort(&[
        ("k_bug2_fcfs_double_queue", "NullPointerException"),
        ("k_bug2_link_double_queue", "NullPointerException"),
        ("k_bug2_fcfs_overwritten_demand", "NullPointerException"),
    ]);
}

/// Configurations the reference cannot run abort like the reference: a component allocated to
/// a nested resource container (SimuLizar 5.2.2 simulates top-level containers only),
/// `simulate_linking_resources = true` without `stream.BYTESIZE` on the payload frame, a
/// response-time monitor re-entered by recursion (MEAS-1.5), and a triggering reconfiguration-time
/// monitor after the stop (MEAS-7.2).
#[test]
fn reference_aborts_are_reproduced() {
    follows_reference_to_abort(&[
        ("l_ref_nested_allocation", "is not simulated"),
        (
            "l_ref_stream_bytesize_missing",
            "missing id stream.BYTESIZE",
        ),
        (
            "l_ref_recursion_external_call",
            "First measurement to the same context",
        ),
        (
            "l_ref_recursion_assembly_op",
            "First measurement to the same context",
        ),
    ]);
}

/// Each `corpus-fuzz/<name>` (reference abort, `reference-partial/`) must abort in simoxide-sim with an
/// error containing the given text, after a trace equal to the reference's partial trace up to
/// the abort (everything before the reference's `stop` or final `finish` line).
fn follows_reference_to_abort(cases: &[(&str, &str)]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus-fuzz");
    let mut bad = Vec::new();
    for &(name, error) in cases {
        let dir = root.join(name);
        let spec = simoxide_sim::RunSpec::load(&dir.join("run.json")).unwrap();
        let cm = simoxide_sim::CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
        let buf = Buf::default();
        let outputs = simoxide_sim::Outputs {
            trace: Some(Box::new(buf.clone())),
            tape: None,
        };
        let r = simoxide_sim::Simulation::new(&cm, spec.sim_config(name), outputs)
            .and_then(|s| s.run());
        let trace = String::from_utf8(buf.0.borrow().clone()).unwrap();
        let reference = corpus::read_maybe_gz(&dir.join("reference-partial/trace.jsonl"))
            .unwrap()
            .expect("reference-partial/trace.jsonl");
        let want: Vec<&str> = reference.lines().collect();
        // without the reference's finish line, and without the stop line and post-stop drain
        // of an abort inside a process (SIM-6.6: simoxide-sim ends the trace at the error)
        let end = want
            .iter()
            .position(|l| l.starts_with("{\"ev\":\"stop\""))
            .unwrap_or(want.len() - 1);
        let want = &want[..end];
        let got: Vec<&str> = trace.lines().collect();
        if let Some(i) = (0..want.len()).find(|&i| got.get(i) != Some(&want[i])) {
            bad.push(format!(
                "{name}: trace differs from the reference at line {}:\n  reference: {}\n  simoxide:  {}",
                i + 1,
                want[i],
                got.get(i).unwrap_or(&"<end of trace>")
            ));
        }
        match r {
            Err(e) if e.message.contains(error) => eprintln!("{name}: simoxide-sim reports: {e}"),
            Err(e) => bad.push(format!("{name}: unexpected error: {e}")),
            Ok(o) => bad.push(format!(
                "{name}: simoxide-sim ran to the end (t_end={} ns); the reference aborts",
                o.end_ns
            )),
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[derive(Clone, Default)]
struct Buf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs one fuzz-found repro of `corpus-fuzz/` through the harness (both modes).
fn repro(name: &str) {
    let mut opts = HarnessOptions::from_env();
    opts.corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus-fuzz");
    opts.filter = Some(name.to_string());
    opts.modes = vec![Mode::TapeReplay, Mode::OwnRng];
    corpus::run_corpus(&mut common::simoxide_sim(), &opts).assert_all_pass();
}

mod common;
