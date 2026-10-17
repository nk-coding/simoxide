//! Random instances of the fuzz metamodel (`fuzz.ecore`) as XMI.

use super::Rng;
use std::fmt::Write;

/// Names (some shared, so that equal names occur), including characters that need escaping.
const NAMES: &[&str] = &[
    "a",
    "b",
    "c",
    "n1",
    "n2",
    "alpha",
    "Beta",
    "gamma delta",
    "x<y",
    "q&a",
    "it's",
    "\"q\"",
    "",
    "Äb",
    "a.b",
];
const TAGS: &[&str] = &["t", "u", "v", "tag one", "T", ""];
const KINDS: &[&str] = &["A", "B", "C"];
const INTS: &[i64] = &[0, 1, 2, 3, -1, 7, 42, 100, -5, 2147483647, -2147483648];
const LONGS: &[i64] = &[
    0,
    1,
    -1,
    3,
    2147483647,
    2147483648,
    -9223372036854775807,
    1 << 40,
    1024,
];
const REALS: &[&str] = &[
    "0.0", "1.0", "2.5", "-0.5", "1.0E10", "0.1", "3.0", "1.0E-5", "-0.0",
];

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

struct N {
    path: String,
    leaf: bool,
    children: Vec<usize>,
}

/// A model: one `Root` with nodes (some `Leaf`s, nested children up to depth 2), items, and
/// cross-references between them. Small: 2–9 top-level nodes.
pub fn generate(rng: &mut Rng) -> String {
    let mut nodes: Vec<N> = Vec::new();
    let mut top = Vec::new();
    let ntop = rng.range(2, 6) as usize;
    for i in 0..ntop {
        let idx = make_node(rng, &mut nodes, format!("//@nodes.{i}"), 0);
        top.push(idx);
    }
    let nitems = rng.range(0, 4) as usize;
    let items: Vec<String> = (0..nitems).map(|i| format!("//@items.{i}")).collect();
    let mut o = String::new();
    let _ = writeln!(o, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = write!(
        o,
        "<fz:Root xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:fz=\"http://qvto-rs/fuzz\""
    );
    if rng.pct(70) {
        let _ = write!(o, " name=\"{}\"", esc(rng.pick(NAMES)));
    }
    let _ = writeln!(o, ">");
    let paths: Vec<String> = nodes.iter().map(|n| n.path.clone()).collect();
    for &t in &top {
        write_node(rng, &mut o, &nodes, t, "nodes", 1, &paths, &items);
    }
    for _ in 0..nitems {
        let _ = write!(o, "  <items");
        if rng.pct(80) {
            let _ = write!(o, " name=\"{}\"", esc(rng.pick(NAMES)));
        }
        if rng.pct(60) {
            let _ = write!(o, " count=\"{}\"", rng.pick(INTS));
        }
        if rng.pct(60) {
            let _ = write!(o, " ratio=\"{}\"", rng.pick(REALS));
        }
        if rng.pct(50) {
            let _ = write!(o, " owner=\"{}\"", rng.pick(&paths));
        }
        let _ = writeln!(o, "/>");
    }
    let _ = writeln!(o, "</fz:Root>");
    o
}

fn make_node(rng: &mut Rng, nodes: &mut Vec<N>, path: String, depth: u32) -> usize {
    let idx = nodes.len();
    nodes.push(N {
        path: path.clone(),
        leaf: rng.pct(30),
        children: Vec::new(),
    });
    if depth < 2 && rng.pct(45) {
        let k = rng.range(1, 3) as usize;
        for i in 0..k {
            let c = make_node(rng, nodes, format!("{path}/@children.{i}"), depth + 1);
            nodes[idx].children.push(c);
        }
    }
    idx
}

#[allow(clippy::too_many_arguments)]
fn write_node(
    rng: &mut Rng,
    o: &mut String,
    nodes: &[N],
    i: usize,
    feature: &str,
    indent: usize,
    paths: &[String],
    items: &[String],
) {
    let pad = "  ".repeat(indent);
    let n = &nodes[i];
    let _ = write!(o, "{pad}<{feature}");
    if n.leaf {
        let _ = write!(o, " xsi:type=\"fz:Leaf\"");
        if rng.pct(70) {
            let _ = write!(o, " size=\"{}\"", rng.pick(INTS));
        }
    }
    if rng.pct(85) {
        let _ = write!(o, " name=\"{}\"", esc(rng.pick(NAMES)));
    }
    if rng.pct(75) {
        let _ = write!(o, " value=\"{}\"", rng.pick(INTS));
    }
    if rng.pct(50) {
        let _ = write!(o, " big=\"{}\"", rng.pick(LONGS));
    }
    if rng.pct(60) {
        let _ = write!(o, " weight=\"{}\"", rng.pick(REALS));
    }
    if rng.pct(50) {
        let _ = write!(o, " flag=\"{}\"", rng.pct(50));
    }
    if rng.pct(60) {
        let _ = write!(o, " kind=\"{}\"", rng.pick(KINDS));
    }
    if rng.pct(50) {
        let k = rng.range(1, 3);
        let refs: Vec<&str> = (0..k).map(|_| rng.pick(paths).as_str()).collect();
        let mut uniq: Vec<&str> = Vec::new();
        for r in refs {
            if !uniq.contains(&r) {
                uniq.push(r);
            }
        }
        let _ = write!(o, " refs=\"{}\"", uniq.join(" "));
    }
    if !items.is_empty() && rng.pct(50) {
        let k = rng.range(1, items.len() as i64) as usize;
        let mut sel: Vec<&str> = Vec::new();
        for _ in 0..k {
            let it = rng.pick(items).as_str();
            if !sel.contains(&it) {
                sel.push(it);
            }
        }
        let _ = write!(o, " items=\"{}\"", sel.join(" "));
    }
    if !items.is_empty() && rng.pct(40) {
        let _ = write!(o, " first=\"{}\"", rng.pick(items));
    }
    let ntags = if rng.pct(50) { rng.range(1, 3) } else { 0 };
    let nnums = if rng.pct(50) { rng.range(1, 4) } else { 0 };
    if ntags == 0 && nnums == 0 && n.children.is_empty() {
        let _ = writeln!(o, "/>");
        return;
    }
    let _ = writeln!(o, ">");
    let mut seen = Vec::new();
    for _ in 0..ntags {
        let t = rng.pick(TAGS);
        // tags is unique: EMF would drop duplicates on load anyway
        if !seen.contains(t) {
            seen.push(t);
            let _ = writeln!(o, "{pad}  <tags>{}</tags>", esc(t));
        }
    }
    for _ in 0..nnums {
        let _ = writeln!(
            o,
            "{pad}  <nums>{}</nums>",
            rng.pick(&[0, 1, 1, 2, 3, -4, 1024])
        );
    }
    for &c in &n.children {
        write_node(rng, o, nodes, c, "children", indent + 1, paths, items);
    }
    let _ = writeln!(o, "{pad}</{feature}>");
}
