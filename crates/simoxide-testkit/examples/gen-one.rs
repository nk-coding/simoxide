//! `cargo run -p simoxide-testkit --example gen-one -- <dir> <seed> <size>`: writes one generated model and
//! prints the simoxide-model diagnostics.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(&a[1]);
    let seed: u64 = a[2].parse().unwrap();
    let size: u32 = a[3].parse().unwrap();
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let m = simoxide_testkit::modelgen::generate(&simoxide_testkit::modelgen::GenConfig::new(
        name, seed, size,
    ));
    simoxide_testkit::modelgen::xmi::write_model(&m, &dir).unwrap();
    let model = simoxide_model::load_dir(&dir).unwrap();
    for d in model.diagnostics.iter() {
        println!("{d}");
    }
    println!("features: {}", m.features.join(", "));
}
