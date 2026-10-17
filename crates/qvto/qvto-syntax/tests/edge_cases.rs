//! Hand-written snippets around the grammar's corners, with Eclipse's parse-only verdict
//! (`tests/fixtures/eclipse-edge-cases.tsv`): we must accept and reject the same ones. This
//! test needs no reference checkout.

#[test]
fn edge_cases_agree_with_eclipse() {
    let text = include_str!("fixtures/eclipse-edge-cases.tsv");
    let mut n = 0;
    let mut bad = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.splitn(3, '\t').collect();
        let src = match f[0] {
            "stmt" => format!("transformation T();\nmain() {{\n{}\n}}\n", f[2]),
            "elem" => format!("transformation T();\n{}\n", f[2]),
            _ => format!("{}\n", f[2]),
        };
        let eclipse_rejects = f[1] != "0";
        let p = qvto_syntax::parse(&src);
        n += 1;
        if p.has_errors() != eclipse_rejects {
            bad.push(format!(
                "{:?}: eclipse {} errors, ours {:?}",
                f[2],
                f[1],
                p.errors().map(|d| d.to_string()).collect::<Vec<_>>()
            ));
        }
    }
    assert!(n > 150);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
