//! Regressions found by the Q7 fuzzing campaigns and the external corpus: every
//! `tests/data/q7/<name>.qvto` must give exactly the compile problems Eclipse gives
//! (`<name>.diag`, recorded with `tools/q7-record.sh`): severity, unit, line, UTF-16 offset and
//! length, message. The files use no metamodel besides Ecore.

use qvto_sema::{UnitResolver, UnitSource, compile};
use std::path::Path;
use std::rc::Rc;

struct NoImports;

impl UnitResolver for NoImports {
    fn resolve(&self, _importer: &UnitSource, _q: &str) -> Option<UnitSource> {
        None
    }
}

fn problems(name: &str, text: &str) -> Vec<String> {
    let mut rs = qvto_emf::resource_set::ResourceSet::new();
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let main = UnitSource {
        key: format!("mem:{name}"),
        qualified_name: name.to_string(),
        file_name: format!("{name}.qvto"),
        path: None,
        text: Rc::from(text),
        blackbox: None,
    };
    let c = compile(main, &NoImports, meta);
    let mut out = Vec::new();
    for u in &c.units {
        for p in &u.problems {
            let (line, o, l) = p.eclipse_position(&u.source.text, &u.lines);
            out.push(format!(
                "problem\t{}\t{}\t{line}\t{o}\t{l}\t{}",
                if p.is_error() { "error" } else { "warning" },
                u.source.file_name,
                p.message.replace('\n', "\\n")
            ));
        }
    }
    out.sort();
    out
}

#[test]
fn eclipse_problems() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/q7");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "qvto"))
        .collect();
    files.sort();
    let mut failures = Vec::new();
    for f in &files {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(f).unwrap();
        let mut expected: Vec<String> = std::fs::read_to_string(f.with_extension("diag"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        expected.sort();
        let actual = problems(&name, &text);
        if actual != expected {
            failures.push(format!("{name}:\n  eclipse {expected:#?}\n  ours {actual:#?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
