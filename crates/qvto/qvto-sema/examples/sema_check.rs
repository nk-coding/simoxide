//! Compiles one `.qvto` file against Ecore (plus `.ecore` files given with `--mm`) and prints
//! the problems: `cargo run --example sema_check -p qvto-sema -- file.qvto [--mm x.ecore]...`.
//! Imports resolve relative to the file's folder (`a.b.C` → `a/b/C.qvto`).

use qvto_sema::{UnitResolver, UnitSource, compile};
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct DirResolver(PathBuf);

fn source(path: &Path, qname: &str) -> Option<UnitSource> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(UnitSource {
        key: path.display().to_string(),
        qualified_name: qname.to_string(),
        file_name: path.file_name()?.to_string_lossy().into_owned(),
        path: Some(path.to_path_buf()),
        text: Rc::from(text),
        blackbox: None,
    })
}

impl UnitResolver for DirResolver {
    fn resolve(&self, _importer: &UnitSource, q: &str) -> Option<UnitSource> {
        let p = self.0.join(format!("{}.qvto", q.replace('.', "/")));
        source(&p, q)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = PathBuf::from(&args[0]);
    let mut rs = qvto_emf::resource_set::ResourceSet::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--mm" {
            let r = rs
                .load_file(Path::new(&args[i + 1]))
                .expect("load metamodel");
            rs.register(r).expect("register");
            i += 2;
        } else {
            i += 1;
        }
    }
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let dir = file.parent().unwrap().to_path_buf();
    let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
    let main = source(&file, &stem).expect("read file");
    let t = std::time::Instant::now();
    let c = compile(main, &DirResolver(dir), meta);
    let el = t.elapsed();
    for u in &c.units {
        for p in &u.problems {
            let (off, len) = p.utf16(&u.source.text);
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                u.source.file_name,
                if p.is_error() { "E" } else { "W" },
                p.line(&u.lines),
                off,
                len,
                p.message
            );
        }
    }
    eprintln!("compiled in {el:?}");
}
