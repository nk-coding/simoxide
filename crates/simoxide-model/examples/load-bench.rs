//! Loading benchmark and output snapshot for the XMI loader
//! (`cargo run --release -p simoxide-model --example load-bench -- ...`).
//!
//! ```text
//! load-bench bench [--reps N] [--phases] [--memory] [--cache|--cache-miss] [--profile F] [dir...]
//! load-bench snapshot OUT_DIR [dir...]
//! ```
//!
//! `bench` prints the time per model (best of N; default dirs: `corpus/*` and
//! `crates/simoxide-cli/bench/models/*`) and the allocations of one load. It loads like `simoxide-sim`
//! does (`SimSpec::load_model`): the `.usagemodel`, `.allocation` and `.monitorrepository` files
//! as entry points, everything else demand-loaded.
//! - `--phases`: also parse + resolve (`Loader::finish`) and typed build + validation.
//! - `--memory`: the same files from memory (`Loader::add_file`, no file system access).
//! - `--cache`: a `ParseCache` shared by the repetitions (every load after the second hits);
//!   `--cache-miss` clears it before each repetition (its cost when nothing repeats).
//! - `--profile out.folded`: sampling profile (needs `--features profile`).
//!
//! `snapshot` writes everything the loader produces (graph, typed model, diagnostics; strict,
//! tolerant and entry-file loading) per model directory (default: as `bench`, plus every
//! directory of the simoxide-model oracle), for before/after comparisons with `diff -r`.
use simoxide_model::{LoadOptions, Loader, canon};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Counts allocations (reported per load).
struct Counting;
thread_local! {
    static ALLOCS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
fn alloc_count() -> u64 {
    ALLOCS.with(|c| c.get())
}
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { std::alloc::System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { std::alloc::System.realloc(p, l, n) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn default_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for base in ["corpus", "crates/simoxide-cli/bench/models"] {
        let Ok(rd) = std::fs::read_dir(root().join(base)) else {
            continue;
        };
        let mut ds: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        ds.sort();
        v.extend(ds);
    }
    v
}

/// Entry files as `simoxide_sim::SimSpec::load_model` picks them without a spec.
fn entry_files(dir: &Path) -> Vec<PathBuf> {
    let list = |ext: &str| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == ext))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let mut files = list("usagemodel");
    files.truncate(1);
    files.extend(list("allocation"));
    files.extend(list("monitorrepository"));
    files
}

fn model_dirs(args: &[String]) -> Vec<PathBuf> {
    if args.is_empty() {
        default_dirs()
    } else {
        args.iter().map(PathBuf::from).collect()
    }
}

fn bench(args: &[String]) {
    let mut reps = 20usize;
    let mut phases = false;
    let mut memory = false;
    let mut cache = false;
    let mut cache_miss = false;
    let mut profile: Option<String> = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--reps" => reps = it.next().and_then(|s| s.parse().ok()).expect("--reps N"),
            "--phases" => phases = true,
            "--memory" => memory = true,
            "--cache" => cache = true,
            "--cache-miss" => {
                cache = true;
                cache_miss = true;
            }
            "--profile" => profile = it.next().cloned(),
            _ => rest.push(a.clone()),
        }
    }
    let dirs = model_dirs(&rest);
    let guard = Profiler::start(profile.is_some());
    let mut total = 0.0;
    let mut total_objs = 0usize;
    let mut total_allocs = 0u64;
    println!(
        "{:<44} {:>7} {:>7} {:>10} {:>10} {:>10}",
        "model", "objects", "allocs", "load µs", "finish µs", "build µs"
    );
    for d in &dirs {
        let files = entry_files(d);
        if files.is_empty() {
            continue;
        }
        let (mut best, mut best_fin, mut best_build) = (f64::MAX, f64::MAX, f64::MAX);
        let mut objs = 0;
        let mut allocs = 0;
        let contents: Vec<(String, String)> = if memory {
            let mut v: Vec<(String, String)> = std::fs::read_dir(d)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file())
                .map(|p| {
                    let n = p.file_name().unwrap().to_string_lossy().into_owned();
                    (n, std::fs::read_to_string(&p).unwrap_or_default())
                })
                .collect();
            v.sort();
            v
        } else {
            Vec::new()
        };
        let mem_dir = Path::new(simoxide_model::load::MEMORY_DIR);
        // a cache shared by the repetitions (all but the first load hit)
        let shared = cache.then(|| std::sync::Arc::new(simoxide_model::ParseCache::default()));
        for _ in 0..reps {
            // the in-memory files arrive as owned strings (not timed)
            let mem = contents.clone();
            let a0 = alloc_count();
            let t0 = Instant::now();
            let mut l = Loader::default();
            if let Some(c) = &shared {
                if cache_miss {
                    c.clear();
                }
                l.set_cache(c.clone());
            }
            if memory {
                l.set_read_files(false);
                for (n, c) in mem {
                    l.add_file(mem_dir.join(n), c);
                }
                l.set_base_dir(mem_dir);
                for f in &files {
                    l.load_file(mem_dir.join(f.file_name().unwrap()));
                }
            } else {
                for f in &files {
                    l.load_file(f);
                }
            }
            let (g, diags) = l.finish();
            let t1 = Instant::now();
            let m = simoxide_model::build_from_graph(g, diags);
            let t2 = Instant::now();
            allocs = alloc_count() - a0;
            objs = m.graph.objs.len();
            best_fin = best_fin.min((t1 - t0).as_secs_f64());
            best_build = best_build.min((t2 - t1).as_secs_f64());
            best = best.min((t2 - t0).as_secs_f64());
            drop(m);
        }
        total += best;
        total_objs += objs;
        total_allocs += allocs;
        let name = d.file_name().unwrap().to_string_lossy();
        if phases {
            println!(
                "{name:<44} {objs:>7} {allocs:>7} {:>10.1} {:>10.1} {:>10.1}",
                best * 1e6,
                best_fin * 1e6,
                best_build * 1e6
            );
        } else {
            println!("{name:<44} {objs:>7} {allocs:>7} {:>10.1}", best * 1e6);
        }
    }
    if let Some(p) = profile {
        guard.write(&p);
    }
    println!(
        "TOTAL {} models, {total_objs} objects, {total_allocs} allocations: {:.1} µs (best of {reps} each)",
        dirs.len(),
        total * 1e6
    );
}

#[cfg(feature = "profile")]
struct Profiler(Option<pprof::ProfilerGuard<'static>>);
#[cfg(feature = "profile")]
impl Profiler {
    fn start(on: bool) -> Self {
        Profiler(on.then(|| pprof::ProfilerGuard::new(4999).unwrap()))
    }
    /// Folded stacks, root first.
    fn write(self, path: &str) {
        use std::io::Write;
        let r = self.0.unwrap().report().build().unwrap();
        let mut f = std::fs::File::create(path).unwrap();
        for (frames, n) in &r.data {
            let mut names: Vec<String> = Vec::new();
            for fr in frames.frames.iter().rev() {
                for sym in fr.iter().rev() {
                    names.push(sym.name());
                }
            }
            writeln!(f, "{} {n}", names.join(";")).unwrap();
        }
    }
}
#[cfg(not(feature = "profile"))]
struct Profiler;
#[cfg(not(feature = "profile"))]
impl Profiler {
    fn start(on: bool) -> Self {
        assert!(!on, "--profile needs the `profile` feature");
        Profiler
    }
    fn write(self, _: &str) {}
}

fn snapshot_one(files: &[PathBuf], dir: Option<&Path>, opts: LoadOptions) -> String {
    let mut l = Loader::new(opts);
    match dir {
        Some(d) => {
            l.load_dir(d).expect("read dir");
        }
        None => {
            for f in files {
                l.load_file(f);
            }
        }
    }
    let m = l.build();
    let mut s = String::new();
    use std::fmt::Write;
    for d in m.diagnostics.iter() {
        writeln!(s, "{d}").unwrap();
    }
    writeln!(s, "{:#?}", m.graph).unwrap();
    s.push_str(&canon::dump(&m.graph, false));
    s.push_str(&canon::typed_dump(&m));
    macro_rules! arenas {
        ($($f:ident),*) => { $( writeln!(s, "{}: {:#?}", stringify!($f), m.$f).unwrap(); )* };
    }
    arenas!(
        repositories,
        components,
        interfaces,
        signatures,
        parameters,
        data_types,
        roles,
        passive_resources,
        seffs,
        behaviours,
        actions,
        structures,
        assembly_contexts,
        connectors,
        systems,
        resource_environments,
        containers,
        processing_resources,
        linking_resources,
        resource_repositories,
        resource_types,
        scheduling_policies,
        resource_interfaces,
        resource_signatures,
        allocations,
        allocation_contexts,
        usage_models,
        usage_scenarios,
        scenario_behaviours,
        user_actions,
        monitor_repositories,
        monitors,
        measuring_points,
        metrics
    );
    s
}

fn snapshot(args: &[String]) {
    let out = PathBuf::from(&args[0]);
    std::fs::create_dir_all(&out).unwrap();
    let mut dirs = model_dirs(&args[1..]);
    if args.len() == 1 {
        // plus every directory of the EMF oracle's model list
        let list = root().join("reference/oracles/pcm-model/models.txt");
        if let Ok(t) = std::fs::read_to_string(list) {
            for l in t.lines().filter(|l| !l.starts_with('#')) {
                if let Some((_, d)) = l.split_once('\t')
                    && d != "-"
                    && Path::new(d).is_dir()
                {
                    dirs.push(PathBuf::from(d));
                }
            }
        }
    }
    for (i, d) in dirs.iter().enumerate() {
        let name = format!(
            "{i:03}_{}",
            d.file_name().unwrap().to_string_lossy().replace('/', "_")
        );
        let mut s = String::new();
        s.push_str("=== dir, strict\n");
        s.push_str(&snapshot_one(&[], Some(d), LoadOptions::strict()));
        s.push_str("=== dir, tolerant\n");
        s.push_str(&snapshot_one(&[], Some(d), LoadOptions::default()));
        let files = entry_files(d);
        if !files.is_empty() {
            s.push_str("=== entry files, tolerant\n");
            s.push_str(&snapshot_one(&files, None, LoadOptions::default()));
        }
        std::fs::write(out.join(name), s).unwrap();
    }
    println!("{} models", dirs.len());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") => bench(&args[1..]),
        Some("snapshot") if args.len() >= 2 => snapshot(&args[1..]),
        _ => {
            eprintln!(
                "usage: load-bench bench [--reps N] [--phases] [--memory] [--cache|--cache-miss] [dir...] | snapshot OUT [dir...]"
            );
            std::process::exit(2);
        }
    }
}
