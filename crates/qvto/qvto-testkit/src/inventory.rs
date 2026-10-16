//! The inventory of the Eclipse QVTo test suite (`reference/inventory/eclipse-tests.tsv`).

use std::path::Path;

/// One row of the inventory.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub test_id: String,
    pub driver: String,
    pub qvto_path: String,
    pub inputs: Vec<String>,
    pub expected_outputs: Vec<String>,
    pub expected_kind: String,
    pub needs_java_blackbox: bool,
    pub metamodels: Vec<String>,
    pub feature_tags: Vec<String>,
    pub notes: String,
}

/// Reads the inventory TSV (header line, tab-separated columns).
pub fn read_inventory(path: &Path) -> Result<Vec<Row>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_inventory(&text)
}

pub fn parse_inventory(text: &str) -> Result<Vec<Row>, String> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().ok_or("empty inventory")?.split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name).ok_or(format!("no column {name}"));
    let (id, drv, q, i, e, k, bb, mm, ft, no) = (
        col("test_id")?,
        col("driving_test_class")?,
        col("qvto_path")?,
        col("inputs")?,
        col("expected_outputs")?,
        col("expected_kind")?,
        col("needs_java_blackbox")?,
        col("metamodels")?,
        col("feature_tags")?,
        col("notes")?,
    );
    let list = |s: &str| -> Vec<String> { s.split(',').filter(|x| !x.is_empty()).map(String::from).collect() };
    let mut rows = Vec::new();
    for l in lines {
        if l.is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split('\t').collect();
        let get = |c: usize| f.get(c).copied().unwrap_or("");
        rows.push(Row {
            test_id: get(id).into(),
            driver: get(drv).into(),
            qvto_path: get(q).into(),
            inputs: list(get(i)),
            expected_outputs: list(get(e)),
            expected_kind: get(k).into(),
            needs_java_blackbox: get(bb) == "y",
            metamodels: list(get(mm)),
            feature_tags: list(get(ft)),
            notes: get(no).into(),
        });
    }
    Ok(rows)
}
