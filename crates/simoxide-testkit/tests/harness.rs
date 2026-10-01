//! Corpus harness and fuzz pipeline tests with stand-in simulators. Tests that start the Java
//! reference are `#[ignore]` (run with `cargo test -p simoxide-testkit -- --ignored`).

use std::sync::Arc;

use simoxide_testkit::corpus::{self, HarnessOptions, Mode, Outcome};
use simoxide_testkit::fuzz::{self, FuzzOptions};
use simoxide_testkit::sim::{
    FnSim, RefSim, RngMode, RunOutput, RunRequest, SimError, Simulator, Unimplemented,
};

/// A "simulator" that returns the expected files of the corpus entry (consuming the tape in
/// replay mode with origin checks, like a real port would).
fn echo(req: &RunRequest) -> Result<RunOutput, SimError> {
    let e = corpus::Entry {
        name: req.name.clone(),
        dir: req.model_dir.clone(),
        config: req.config.clone(),
    }
    .expected()
    .map_err(|e| SimError::Io(e.to_string()))?;
    if let RngMode::TapeReplay(t) = &req.rng {
        let mut r = t.replay();
        for i in 0..t.len() {
            let o = t.origin(i).to_string();
            r.next_checked(&o)
                .map_err(|e| SimError::Failed(e.to_string()))?;
        }
        assert_eq!(r.remaining(), 0);
    }
    Ok(RunOutput {
        trace: e.trace,
        tape: e.tape,
        measurements: e.measurements,
        wall: Default::default(),
    })
}

fn opts(filter: &str) -> HarnessOptions {
    HarnessOptions {
        filter: Some(filter.into()),
        ..HarnessOptions::default()
    }
}

#[test]
fn echo_simulator_passes() {
    let mut sim = FnSim::new("echo", echo);
    let r = corpus::run_corpus(&mut sim, &opts("h0,x_ss_minimal"));
    assert!(r.cases.len() >= 20, "{}", r.table());
    assert_eq!(r.failed(), 0, "{}", r.table());
    r.assert_all_pass();
}

#[test]
fn perturbed_simulator_fails_with_first_divergence() {
    let mut sim = FnSim::new("perturbed", |req: &RunRequest| {
        let mut o = echo(req)?;
        if let Some(t) = o.trace.as_mut() {
            // change the first demand's service time in the 2nd half of the trace
            let lines: Vec<&str> = t.lines().collect();
            let k = lines.len() / 2
                + lines[lines.len() / 2..]
                    .iter()
                    .position(|l| l.contains("\"ev\":\"demand\""))
                    .unwrap();
            let mut v: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
            v[k] = v[k].replace("\"st\":", "\"st\":1");
            *t = v.join("\n") + "\n";
        }
        if matches!(req.rng, RngMode::OwnRng) {
            o.measurements = o.measurements.replacen(",0.", ",1.", 1);
        }
        Ok(o)
    });
    let r = corpus::run_corpus(&mut sim, &opts("h01_ps_single"));
    assert_eq!(r.failed(), 2, "{}", r.table());
    let t = r.table();
    assert!(t.contains("FAIL") && t.contains("trace line"), "{t}");
    let Outcome::Fail(f) = &r.cases[0].outcome else {
        panic!()
    };
    let d = f.trace.as_ref().unwrap().divergence.as_ref().unwrap();
    assert_eq!(d.fields[0].key, "st");
    assert!(d.process.is_some() && !d.process_stack.is_empty());
    let own = r.cases.iter().find(|c| c.mode == Mode::OwnRng).unwrap();
    let Outcome::Fail(f) = &own.outcome else {
        panic!()
    };
    assert!(f.measurements.is_some());
    let p = std::panic::catch_unwind(|| r.assert_all_pass());
    assert!(p.is_err());
}

#[test]
fn unimplemented_is_skipped() {
    let r = corpus::run_corpus(&mut Unimplemented, &opts("h02"));
    assert_eq!(r.skipped(), 2);
    r.assert_all_pass();
    let strict = HarnessOptions {
        strict: true,
        ..opts("h02")
    };
    let r = corpus::run_corpus(&mut Unimplemented, &strict);
    assert!(std::panic::catch_unwind(|| r.assert_all_pass()).is_err());
}

#[test]
fn replay_origin_mismatch_is_reported() {
    let mut sim = FnSim::new("wrong-order", |req: &RunRequest| {
        if let RngMode::TapeReplay(t) = &req.rng {
            let mut r = t.replay();
            r.next_checked("interarrival")
                .map_err(|e| SimError::Failed(e.to_string()))?;
            r.next_checked("think")
                .map_err(|e| SimError::Failed(e.to_string()))?;
        }
        Err(SimError::Unimplemented)
    });
    let r = corpus::run_corpus(
        &mut sim,
        &HarnessOptions {
            modes: vec![Mode::TapeReplay],
            ..opts("h01_ps_single")
        },
    );
    let Outcome::Fail(f) = &r.cases[0].outcome else {
        panic!("{}", r.table())
    };
    let msg = f.error.as_ref().unwrap().to_string();
    assert!(
        msg.contains("uniform #1") && msg.contains("demand:"),
        "{msg}"
    );
    let _ = Arc::new(0);
}

#[test]
#[ignore]
fn refsim_reproduces_corpus_expected() {
    let mut sim = RefSim::new();
    let o = HarnessOptions {
        modes: vec![Mode::OwnRng],
        ..opts("h01_ps_single,h13_passive,h26,x_ss_minimal,x_pem_fork")
    };
    let r = corpus::run_corpus(&mut sim, &o);
    assert!(r.cases.len() >= 5);
    r.assert_all_pass();
}

/// The whole fuzz pipeline with the reference as its own candidate (no divergence), then with a
/// candidate that corrupts one measurement (divergence saved and minimized).
#[test]
#[ignore]
fn fuzz_pipeline() {
    let out = std::env::temp_dir().join(format!("testkit-fuzz-out-{}", std::process::id()));
    let o = FuzzOptions {
        n: 4,
        start_seed: 100,
        sizes: (2, 5),
        modes: vec![Mode::OwnRng],
        out_dir: out.clone(),
        max_minimize_rounds: 1,
        ..FuzzOptions::default()
    };
    let mut refsim = RefSim::new();
    let mut cand = RefSim::new();
    let rep = fuzz::run_fuzz(&mut refsim, &mut cand, &o);
    assert_eq!(rep.reference_ok, 4, "{rep}");
    assert!(rep.divergences.is_empty(), "{rep}");

    let mut inner = RefSim::new();
    let mut bad = FnSim::new("bad", move |req: &RunRequest| {
        let mut x = inner.run(req)?;
        x.measurements = x.measurements.replacen(",0.", ",7.", 1);
        Ok(x)
    });
    let o = FuzzOptions { n: 1, ..o };
    let rep = fuzz::run_fuzz(&mut refsim, &mut bad, &o);
    assert_eq!(rep.divergences.len(), 1, "{rep}");
    let d = &rep.divergences[0];
    let saved = d.saved.as_ref().unwrap();
    assert!(
        saved.join("expected/measurements.csv").exists()
            || saved.join("expected/measurements.csv.gz").exists()
    );
    assert!(saved.join("REPORT.own-rng.txt").exists() && saved.join("run.json").exists());
    let m = d.minimized.as_ref().expect("minimized");
    assert!(out.join(&m.name).join("run.json").exists());
    // the saved case is a valid corpus entry: the reference reproduces it
    let r = corpus::run_corpus(
        &mut refsim,
        &HarnessOptions {
            corpus: out.clone(),
            modes: vec![Mode::OwnRng],
            ..HarnessOptions::default()
        },
    );
    r.assert_all_pass();
    let _ = std::fs::remove_dir_all(&out);
}
