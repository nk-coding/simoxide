//! Debug tool: `pcm-dump [--canon] [--typed] [--strict] [--quiet] <model dir or files...>`
//!
//! Loads the files (all model files of a directory), resolves references, builds and validates
//! the typed model and prints the diagnostics and a summary. `--canon` prints the canonical
//! EMF-comparable dump of the object graph instead, `--typed` the typed model (Debug format).
//! `--strict` disables the tolerant fallbacks (EMF behaviour).
use simoxide_model::{LoadOptions, Loader, canon};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut canon_mode, mut typed, mut quiet) = (false, false, false);
    let mut opts = LoadOptions::default();
    args.retain(|a| match a.as_str() {
        "--canon" => {
            canon_mode = true;
            false
        }
        "--typed" => {
            typed = true;
            false
        }
        "--quiet" => {
            quiet = true;
            false
        }
        "--strict" => {
            opts = LoadOptions::strict();
            false
        }
        _ => true,
    });
    if args.is_empty() {
        eprintln!(
            "usage: pcm-dump [--canon] [--typed] [--strict] [--quiet] <model dir or files...>"
        );
        std::process::exit(2);
    }
    let t0 = std::time::Instant::now();
    let mut l = Loader::new(opts);
    for a in &args {
        let p = std::path::Path::new(a);
        if a.contains("://") {
            l.load_uri(a);
        } else if p.is_dir() {
            l.load_dir(p).expect("cannot read directory");
        } else {
            l.load_file(p);
        }
    }
    if canon_mode {
        let (g, _) = l.finish();
        print!(
            "{}",
            canon::dump(&g, args.iter().all(|a| a.contains("://")))
        );
        return;
    }
    let m = l.build();
    let dt = t0.elapsed();
    if typed {
        println!("{:#?}", m.repositories);
        println!("{:#?}", m.components);
        println!("{:#?}", m.seffs);
        println!("{:#?}", m.behaviours);
        println!("{:#?}", m.actions);
        println!("{:#?}", m.systems);
        println!("{:#?}", m.structures);
        println!("{:#?}", m.assembly_contexts);
        println!("{:#?}", m.connectors);
        println!("{:#?}", m.containers);
        println!("{:#?}", m.processing_resources);
        println!("{:#?}", m.linking_resources);
        println!("{:#?}", m.allocation_contexts);
        println!("{:#?}", m.usage_scenarios);
        println!("{:#?}", m.user_actions);
        println!("{:#?}", m.monitors);
        println!("{:#?}", m.measuring_points);
    }
    if !quiet {
        for d in m.diagnostics.iter() {
            println!("{d}");
        }
    }
    println!(
        "{} resources, {} objects; {} components, {} seffs, {} actions, {} assembly contexts, {} containers, {} usage scenarios, {} monitors; {} errors, {} warnings; {:.3} ms",
        m.graph.resources.len(),
        m.graph.objs.len(),
        m.components.len(),
        m.seffs.len(),
        m.actions.len(),
        m.assembly_contexts.len(),
        m.containers.len(),
        m.usage_scenarios.len(),
        m.monitors.len(),
        m.diagnostics.count(simoxide_model::Level::Error),
        m.diagnostics.count(simoxide_model::Level::Warning),
        dt.as_secs_f64() * 1e3
    );
}
