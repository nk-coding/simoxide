//! Generates `src/stdlib_data.rs` from `data/stdlib.tsv` (the dump of Eclipse's standard library
//! made by `tools/oracle/StdlibDump.java`).
//!
//! ```sh
//! cargo run -p qvto-types --example gen_stdlib            # writes src/stdlib_data.rs
//! cargo run -p qvto-types --example gen_stdlib -- --check # fails if out of date
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

struct Row {
    section: String,
    owner: String,
    name: String,
    params: Vec<(String, String)>,
    ret: String,
    flags: String,
}

fn owner_key(owner: &str) -> Option<&'static str> {
    Some(match owner {
        "oclstdlib::OclAny" => "OclAny",
        "oclstdlib::OclVoid" => "OclVoid",
        "oclstdlib::OclInvalid" => "OclInvalid",
        "oclstdlib::Boolean" => "Boolean",
        "oclstdlib::Integer" => "Integer",
        "oclstdlib::Real" => "Real",
        "oclstdlib::String" => "String",
        "oclstdlib::UnlimitedNatural" => "UnlimitedNatural",
        "oclstdlib::OclMessage" => "OclMessage",
        "TypeType(oclstdlib::T)" => "OclType",
        "Collection(oclstdlib::T)" => "Collection",
        "Set(oclstdlib::T)" => "Set",
        "OrderedSet(oclstdlib::T)" => "OrderedSet",
        "Bag(oclstdlib::T)" => "Bag",
        "Sequence(oclstdlib::T)" => "Sequence",
        "List(oclstdlib::T)" => "List",
        "Dict(Stdlib::KeyT,oclstdlib::T)" => "Dict",
        "Stdlib::Element" => "Element",
        "Stdlib::Exception" => "Exception",
        "Stdlib::Model" => "Model",
        "Stdlib::Status" => "Status",
        "Stdlib::Transformation" => "Transformation",
        "Stdlib::Stdlib" => "Stdlib",
        _ => return None,
    })
}

fn op_word(name: &str, nparams: usize) -> String {
    match name {
        "=" => "eq".into(),
        "<>" => "ne".into(),
        "<" => "lt".into(),
        ">" => "gt".into(),
        "<=" => "le".into(),
        ">=" => "ge".into(),
        "+" => "add".into(),
        "-" if nparams == 0 => "neg".into(),
        "-" => "sub".into(),
        "*" => "mul".into(),
        "/" => "divide".into(),
        _ => name.into(),
    }
}

fn short_type(t: &str) -> String {
    let t = t
        .replace("oclstdlib::", "")
        .replace("Stdlib::", "")
        .replace("TypeType", "Type");
    t.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("")
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tsv = std::fs::read_to_string(root.join("data/stdlib.tsv")).unwrap();
    let mut rows = Vec::new();
    for line in tsv.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] != "op" {
            continue;
        }
        let params = if f[4].is_empty() {
            Vec::new()
        } else {
            f[4].split(';')
                .map(|p| {
                    let (n, t) = p.split_once(':').unwrap();
                    (n.to_string(), t.to_string())
                })
                .collect()
        };
        rows.push(Row {
            section: f[1].into(),
            owner: f[2].into(),
            name: f[3].into(),
            params,
            ret: f[5].into(),
            flags: f.get(6).copied().unwrap_or("").into(),
        });
    }

    // Operations: OCL ones deduplicated by signature (classic OCL evaluates by name and the
    // source value); QVTo ones (handlers) per owner.
    let mut ops: Vec<(String, &Row)> = Vec::new(); // (variant, row)
    let mut by_sig: HashMap<String, usize> = HashMap::new();
    let mut owner_lists: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new(); // (section, owner)
    let mut used_names: HashMap<String, usize> = HashMap::new();
    // Name operations after the most general owner that has them: Real before Integer before
    // UnlimitedNatural (a stable sort of the rows by owner rank), lists in dump order below.
    let rank = |o: &str| match o {
        "Real" => 0,
        "Integer" => 1,
        "UnlimitedNatural" => 2,
        _ => 0,
    };
    let mut naming: Vec<&Row> = rows.iter().collect();
    naming.sort_by_key(|r| owner_key(&r.owner).map(rank).unwrap_or(0));
    let mut row_op: HashMap<*const Row, usize> = HashMap::new();
    for r in naming {
        let Some(owner) = owner_key(&r.owner) else {
            continue;
        };
        let section: &str = match r.section.as_str() {
            "ocl" => "ocl",
            "qvto" => "qvto",
            "class" | "module" => "class",
            _ => unreachable!(),
        };
        let sig = format!(
            "{}|{}|{}|{}|{}",
            if section == "ocl" { "ocl" } else { owner },
            r.name,
            r.params
                .iter()
                .map(|p| p.1.as_str())
                .collect::<Vec<_>>()
                .join(","),
            r.ret,
            r.flags
        );
        let idx = if let Some(&i) = by_sig.get(&sig) {
            i
        } else {
            let prefix = if section == "qvto" { "Q" } else { "" };
            let base = format!("{prefix}{owner}_{}", op_word(&r.name, r.params.len()));
            let mut name = base.clone();
            if used_names.contains_key(&name) {
                let suffix: Vec<String> = r.params.iter().map(|p| short_type(&p.1)).collect();
                name = format!("{base}_{}", suffix.join("_"));
                if suffix.is_empty() || used_names.contains_key(&name) {
                    name = format!("{name}_{}", short_type(&r.ret));
                }
            }
            assert!(!used_names.contains_key(&name), "duplicate {name}");
            used_names.insert(name.clone(), ops.len());
            ops.push((name, r));
            by_sig.insert(sig, ops.len() - 1);
            ops.len() - 1
        };
        row_op.insert(r as *const Row, idx);
    }
    for r in &rows {
        let Some(owner) = owner_key(&r.owner) else {
            continue;
        };
        let section: &str = match r.section.as_str() {
            "ocl" => "ocl",
            "qvto" => "qvto",
            _ => "class",
        };
        owner_lists
            .entry((section, owner))
            .or_default()
            .push(row_op[&(r as *const Row)]);
    }

    let mut out = String::new();
    out.push_str(
        "// @generated by `cargo run -p qvto-types --example gen_stdlib` from data/stdlib.tsv\n\
         // (Eclipse QVTo 3.11's standard library, dumped by tools/oracle/StdlibDump.java). Do not edit.\n\n\
         use super::{StdOpDesc, StdOwner, Section};\n\n",
    );
    out.push_str(
        "/// A standard library operation. Variants are named `<Owner>_<name>[_<param types>]`;\n\
         /// a `Q` prefix marks QVTo's additional operations (Java handlers) on OCL types.\n\
         #[allow(non_camel_case_types)]\n\
         #[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]\n\
         #[repr(u16)]\npub enum StdOp {\n",
    );
    for (name, _) in &ops {
        writeln!(out, "    {name},").unwrap();
    }
    out.push_str("}\n\n");
    writeln!(out, "pub(crate) const COUNT: usize = {};\n", ops.len()).unwrap();
    out.push_str("pub(crate) const ALL: &[StdOp] = &[\n");
    for (name, _) in &ops {
        writeln!(out, "    StdOp::{name},").unwrap();
    }
    out.push_str("];\n\n");
    out.push_str("pub(crate) static DESCS: &[StdOpDesc] = &[\n");
    for (name, r) in &ops {
        let params: Vec<String> = r
            .params
            .iter()
            .map(|(n, t)| format!("({n:?}, {t:?})"))
            .collect();
        let mut static_ = false;
        let mut deprecated = false;
        let mut deprecated_by = None;
        let mut unsupported = None;
        for f in r.flags.split(',').filter(|f| !f.is_empty()) {
            if f == "static" {
                static_ = true;
            } else if f == "deprecated" {
                deprecated = true;
            } else if let Some(b) = f.strip_prefix("deprecatedBy=") {
                deprecated = true;
                deprecated_by = Some(b.to_string());
            } else if let Some(u) = f.strip_prefix("unsupported=") {
                unsupported = Some(u.to_string());
            }
        }
        let section = match r.section.as_str() {
            "ocl" => "Section::Ocl",
            "qvto" => "Section::Qvto",
            _ => "Section::Class",
        };
        writeln!(
            out,
            "    StdOpDesc {{ op: StdOp::{name}, section: {section}, owner: StdOwner::{}, name: {:?}, params: &[{}], ret: {:?}, is_static: {static_}, deprecated: {deprecated}, deprecated_by: {:?}, unsupported: {:?} }},",
            owner_key(&r.owner).unwrap(),
            r.name,
            params.join(", "),
            r.ret,
            deprecated_by,
            unsupported,
        )
        .unwrap();
    }
    out.push_str("];\n\n");
    out.push_str(
        "/// Operations per (section, owner), in Eclipse's order.\n\
         pub(crate) static OWNER_OPS: &[(Section, StdOwner, &[StdOp])] = &[\n",
    );
    // keep Eclipse's owner order of the dump: iterate rows in order for first appearance
    let mut seen = Vec::new();
    for r in &rows {
        let Some(owner) = owner_key(&r.owner) else {
            continue;
        };
        let section: &str = match r.section.as_str() {
            "ocl" => "ocl",
            "qvto" => "qvto",
            _ => "class",
        };
        if seen.contains(&(section, owner)) {
            continue;
        }
        seen.push((section, owner));
        let list = &owner_lists[&(section, owner)];
        let names: Vec<String> = list
            .iter()
            .map(|&i| format!("StdOp::{}", ops[i].0))
            .collect();
        let sec = match section {
            "ocl" => "Section::Ocl",
            "qvto" => "Section::Qvto",
            _ => "Section::Class",
        };
        writeln!(
            out,
            "    ({sec}, StdOwner::{owner}, &[{}]),",
            names.join(", ")
        )
        .unwrap();
    }
    out.push_str("];\n");

    let target = root.join("src/stdlib_data.rs");
    if std::env::args().any(|a| a == "--check") {
        let cur = std::fs::read_to_string(&target).unwrap_or_default();
        if cur != out {
            eprintln!("src/stdlib_data.rs is out of date");
            std::process::exit(1);
        }
        return;
    }
    std::fs::write(&target, out).unwrap();
    eprintln!("wrote {} ({} operations)", target.display(), ops.len());
}
