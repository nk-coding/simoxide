//! Debugging aid: `cargo run -p qvto-emf --example emf-tool -- <conf> <model> [save|dump|errors]`
//! (paths relative to the repository root).
use qvto_emf::canon::dump;
use qvto_emf::config::Config;
use qvto_emf::xmi_save::{save, SaveOptions};
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let config = Config::read(&root.join(&a[1]), &root).unwrap();
    let model = root.join(&a[2]);
    let mut rs = config.resource_set_for(&model);
    let r = match rs.load_file(&model) {
        Ok(r) => r,
        Err(e) => {
            println!("load failed: {e}");
            for (u, e) in &rs.failed {
                println!("  {u}: {e}");
            }
            return;
        }
    };
    for e in &rs.graph.resource(r).errors {
        println!("error: {e}");
    }
    for (u, e) in &rs.failed {
        println!("failed: {u}: {e}");
    }
    match a.get(3).map(|s| s.as_str()).unwrap_or("errors") {
        "save" => print!("{}", save(&rs.graph, r, &SaveOptions::for_resource(rs.graph.resource(r))).unwrap_or_else(|e| e)),
        "dump" => {
            rs.resolve_all();
            print!("{}", dump(&rs.graph, &[r]));
        }
        _ => {}
    }
}
