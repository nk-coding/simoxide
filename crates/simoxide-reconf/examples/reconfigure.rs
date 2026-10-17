//! Runs a model, applies QVTo rules at a point in time, and runs on: the QVTo host end to end.
//!
//! ```sh
//! cargo run --release -p simoxide-reconf --example reconfigure -- <model dir> <rule.qvto | rule folder> <time s>
//! ```
//!
//! The model directory is loaded like `simoxide run --model` (its `run.json` if present). A
//! folder runs its rules in file-name order, as SimuLizar's reconfiguration does; every model
//! parameter is bound SimuLizar's way (by the metamodel's namespace URI).

use simoxide_reconf::{Imports, Invocation, QvtoCore, QvtoRun, RuleSet, RunConfig};
use simoxide_sim::{CompiledModel, Outputs, Pause, RunSpec, Simulation};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, rules, at] = &args[..] else {
        eprintln!("usage: reconfigure <model dir> <rule.qvto | rule folder> <time s>");
        std::process::exit(2);
    };
    let dir = PathBuf::from(dir);
    let spec = match RunSpec::load(&dir.join("run.json")) {
        Ok(s) => s,
        Err(_) => RunSpec::default(),
    };
    let cm = CompiledModel::compile(spec.load_model(&dir)?)?;
    let at_s: f64 = at.parse()?;

    let core = QvtoCore::new()?;
    let rules = PathBuf::from(rules);
    let set = if rules.is_dir() {
        RuleSet::load_dir(&core, &rules)?
    } else {
        RuleSet {
            rules: vec![core.compile_file(&rules, &Imports::Folder)?],
        }
    };
    let mut qvto = QvtoRun::new(&core, RunConfig::default())?;
    println!("QVTo mode: {:?}", qvto.mode());

    let mut sim = Simulation::new(&cm, spec.sim_config("reconfigure"), Outputs::default())?;
    sim.run_until(Pause::After((at_s * 1e9) as i64))?;
    let done = match qvto.run_rules(&mut sim, &set, &Invocation::simulizar()) {
        Ok(d) => d,
        Err((d, e)) => {
            eprintln!("error after {} rule(s): {e}", d.rules.len());
            d
        }
    };
    for o in &done.rules {
        println!(
            "{}: {} ({} changes, {} µs){}",
            o.transformation,
            if o.succeeded {
                "applied"
            } else {
                "not applied"
            },
            o.journal.changes.len(),
            o.run_micros,
            o.change
                .as_ref()
                .map(|c| format!(", model generation {}: {:?}", c.generation, c.applied))
                .unwrap_or_default()
        );
        print!("{}", o.log);
    }
    match sim.finish() {
        Ok(r) => println!("t_end={} events={}", r.end_ns as f64 / 1e9, r.events),
        Err(e) => println!("the run failed: {e}"),
    }
    Ok(())
}
