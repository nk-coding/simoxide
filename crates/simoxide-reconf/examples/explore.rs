//! Fork-based exploration (`DESIGN-SPD.md` N2): run the MediaStore to a pause point, write a
//! snapshot, fork it into variants with different change sets (on parallel threads: the
//! restores are independent), run each to the end and compare their key metrics.
//!
//! ```sh
//! cargo run --release -p simoxide-reconf --example explore [-- MODEL_DIR PAUSE_S END_S]
//! ```

use simoxide_reconf::{ChangeFile, QvtoCore, ResumeChanges, Variant, compare, explore};
use simoxide_sim::ir::metric_names;
use simoxide_sim::snapshot::Snapshot;
use simoxide_sim::{CompiledModel, Fast, Mode, Outputs, Pause, RunSpec, SimuLizar, Simulation};
use std::path::PathBuf;

const VARIANTS: &[(&str, &str)] = &[
    ("base", ""),
    // the database server's disk twice as fast
    ("fast-disk", "rate DatabaseServer:HDD 1200 * 1000 * 1000\n"),
    // twice the arrival rate
    ("more-load", "arrival-rate RealisticUsageScenario 0.08\n"),
    // twice the load on a faster application server
    (
        "load+cpu",
        "arrival-rate RealisticUsageScenario 0.08\nrate ApplicationServer:CPU 2 * 1000 * 1000 * 1000\n",
    ),
    // the file storage moved to a new server with a faster disk
    (
        "new-db",
        "add-container DatabaseServer _db2 DatabaseServer2\n\
         rate _db2:HDD 2000 * 1000 * 1000\n\
         allocate FileStorage _db2 _alloc_fs2\n",
    ),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("corpus/x_pem_screencast_ms"));
    let pause_s: f64 = args.get(1).map_or(Ok(2000.0), |s| s.parse())?;
    let end_s: i64 = args.get(2).map_or(Ok(20000), |s| s.parse())?;

    let spec = RunSpec::load(&dir.join("run.json"))?;
    let cm = CompiledModel::compile(spec.load_model(&dir)?)?;
    let mut cfg = spec.sim_config("explore");
    cfg.mode = Mode::Fast;
    cfg.max_measurements = -1;
    cfg.max_sim_time = end_s;

    // 1. run to the pause point and write the snapshot (to memory here; a file works the same)
    let mut sim = Simulation::<Fast>::create(&cm, cfg, Outputs::default())?;
    sim.run_until(Pause::After((pause_s * 1e9) as i64))?;
    let mut bytes = Vec::new();
    sim.snapshot_to(&mut bytes)?;
    let snapshot = Snapshot::read(&bytes[..])?;
    println!(
        "snapshot at t = {} s, {} events, {} bytes",
        snapshot.header.time_ns as f64 / 1e9,
        snapshot.header.events,
        bytes.len()
    );

    // 2. fork it into variants, each with its change file, run in parallel to the end
    let variants: Vec<Variant> = VARIANTS
        .iter()
        .map(|(name, text)| {
            Ok(Variant {
                name: name.to_string(),
                changes: ResumeChanges::new().file(ChangeFile::parse(text)?),
            })
        })
        .collect::<Result<_, simoxide_reconf::ChangeFileError>>()?;
    let core = QvtoCore::new()?;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(4));
    let t0 = std::time::Instant::now();
    let results = explore::<Fast, SimuLizar>(&cm, &snapshot, &core, &variants, None, threads);

    // 3. compare (measurements of the continuation only)
    print!(
        "{}",
        compare(
            &results,
            &[
                metric_names::RESPONSE_TIME,
                metric_names::STATE_OF_ACTIVE_RESOURCE
            ]
        )
    );
    println!(
        "{} variants on {threads} threads in {:.1} ms",
        results.len(),
        t0.elapsed().as_secs_f64() * 1e3
    );
    Ok(())
}
