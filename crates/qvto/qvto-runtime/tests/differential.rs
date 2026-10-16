//! Differential tests against Java (`tools/oracle/run.sh`, needs a JDK and the reference
//! runner's jars). Ignored by default; run with
//!
//! ```sh
//! nice -n 19 cargo test -p qvto-runtime --test differential -- --ignored --nocapture --test-threads=4
//! ```
//!
//! Each test generates requests deterministically, asks Java, evaluates them in Rust and
//! reports the agreement count. `QVTO_DIFF_SCALE` multiplies the case counts;
//! `QVTO_WRITE_GOLDEN=1` writes a sample of the cases with Java's answers to `tests/golden/`.

mod common;

use common::generate::*;
use common::{crate_dir, eval, write_golden, Oracle};

fn scale() -> usize {
    std::env::var("QVTO_DIFF_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1)
}

/// Asks Java, compares, prints the agreement, optionally writes goldens; panics on mismatch.
fn check(name: &str, requests: Vec<String>, golden_sample: usize) {
    let answers = Oracle::start().ask_all(&requests);
    let mut agree = 0usize;
    let mut shown = 0usize;
    for (req, want) in requests.iter().zip(answers.iter()) {
        let got = eval(req);
        if &got == want {
            agree += 1;
        } else if shown < 8 {
            shown += 1;
            eprintln!("MISMATCH {name}\n  request: {req}\n  java:    {want}\n  rust:    {got}");
        }
    }
    println!("{name}: {agree}/{} agree", requests.len());
    if std::env::var("QVTO_WRITE_GOLDEN").is_ok() {
        let step = (requests.len() / golden_sample.max(1)).max(1);
        let pairs: Vec<(String, String)> = requests
            .iter()
            .zip(answers.iter())
            .step_by(step)
            .map(|(a, b)| (a.clone(), b.clone()))
            .collect();
        write_golden(
            &crate_dir().join(format!("tests/golden/{name}.tsv")),
            &format!("{name}: request<TAB>Java answer (tools/oracle, JDK 21). Regenerate: QVTO_WRITE_GOLDEN=1, see tests/differential.rs"),
            &pairs,
        );
    }
    assert_eq!(agree, requests.len(), "{name}: {} mismatches", requests.len() - agree);
}

const SIZES: &[usize] = &[0, 1, 2, 11, 12, 13, 23, 24, 25, 47, 48, 49, 95, 96, 97, 191, 192, 193, 383, 384, 385, 768, 769];

fn values(rng: &mut Rng, objs: &mut Vec<(u32, i32)>, pool: u64, n: usize) -> Vec<String> {
    (0..n)
        .map(|_| match pool {
            0 => format!("i{}", rng.range(-5000, 5000)),
            1 => enc_str(&utf16(rng, 5)),
            2 => obj(rng, objs),
            3 => format!("d{:016x}", double_bits(rng)),
            _ => scalar(rng, objs),
        })
        .collect()
}

fn growth_case(rng: &mut Rng) -> String {
    let mut objs = Vec::new();
    let n = if rng.chance(0.6) { *rng.pick(SIZES) } else { rng.below(300) as usize };
    let pool = rng.below(5);
    let kind = *rng.pick(&["S", "S", "S", "B", "O"]);
    let vals = values(rng, &mut objs, pool, n);
    let mut s = format!("new {kind} 0");
    for (i, v) in vals.iter().enumerate() {
        s += &format!(";+ 0 {v}");
        if rng.chance(0.1) {
            let j = rng.below(i as u64 + 1) as usize;
            s += &format!(";- 0 {}", vals[j]);
        }
        if kind == "B" && rng.chance(0.2) {
            s += &format!(";+ 0 {v}");
        }
    }
    if kind != "B" && rng.chance(0.3) {
        s += &format!(";ret 0 {}", rng.range(2, 5));
    }
    s += ";p 0;cp S 1 0;p 1;cp O 2 0;p 2;cp B 3 0;p 3;cp Q 4 0;p 4";
    if kind == "S" {
        s += ";ph 0";
    }
    if rng.chance(0.3) {
        s += ";clr 0";
        let m = rng.below(30) as usize;
        for v in values(rng, &mut objs, pool, m) {
            s += &format!(";+ 0 {v}");
        }
        s += ";p 0";
    }
    s
}

fn collision_case(rng: &mut Rng) -> String {
    let mut objs = Vec::new();
    let mut s = String::from("new S 0");
    let base = rng.range(30, 120) as usize;
    let filler = values(rng, &mut objs, 0, base);
    let colliders: Vec<String> = match rng.below(5) {
        0 => {
            let (b, c) = (rng.range(4, 6) as u32, rng.range(9, 40) as usize);
            colliding_strings(rng, b, c)
        }
        1 => {
            let c = rng.below(1024) as i64;
            (0..rng.range(9, 40)).map(|k| format!("i{}", c + 1024 * k)).collect()
        }
        2 => {
            let c = rng.below(1024) as i64;
            (0..rng.range(9, 30)).flat_map(|k| [format!("i{}", c + 1024 * k), format!("l{}", c + 1024 * k)]).collect()
        }
        3 => {
            // objects: distinct identity hashes in one bucket
            let c = rng.below(1024) as i32;
            (0..rng.range(9, 40))
                .map(|_| {
                    let id = objs.len() as u32 + 1;
                    let h = ((rng.next() as u32 & 0x1f_ffff) as i32) << 10 | c;
                    objs.push((id, h));
                    format!("o{id}.{h}")
                })
                .collect()
        }
        _ => {
            let mut v = colliding_strings(rng, 5, 20);
            let c = rng.below(1024) as i64;
            v.extend((0..rng.range(5, 20)).map(|k| format!("i{}", c + 1024 * k)));
            v
        }
    };
    let mut all: Vec<&String> = filler.iter().chain(colliders.iter()).collect();
    if rng.chance(0.5) {
        for i in (1..all.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            all.swap(i, j);
        }
    }
    for v in &all {
        s += &format!(";+ 0 {v}");
    }
    s += ";p 0";
    // removals: movable (remove) and through the iterator
    for _ in 0..rng.below(25) {
        s += &format!(";- 0 {}", rng.pick(&colliders));
    }
    s += ";p 0";
    if rng.chance(0.5) {
        s += &format!(";ret 0 {}", rng.range(2, 4));
        s += ";p 0";
    }
    for _ in 0..rng.below(10) {
        s += &format!(";+ 0 {}", rng.pick(&colliders));
    }
    s += ";p 0;cp S 1 0;p 1;new S 2";
    for v in colliders.iter().take(rng.below(colliders.len() as u64 + 1) as usize) {
        s += &format!(";+ 2 {v}");
    }
    s += ";rall 0 2;p 0";
    s
}

fn small_coll(rng: &mut Rng, objs: &mut Vec<(u32, i32)>, reg: usize, kind: &str, pool: &[String]) -> String {
    let mut s = format!("new {kind} {reg}");
    for _ in 0..rng.below(8) {
        s += &format!(";+ {reg} {}", rng.pick(pool));
    }
    let _ = objs;
    s
}

fn ops_case(rng: &mut Rng) -> String {
    let mut objs = Vec::new();
    let numeric = rng.chance(0.3);
    let pool: Vec<String> = (0..rng.range(3, 12))
        .map(|_| {
            if numeric {
                match rng.below(3) {
                    0 => format!("i{}", rng.range(-5, 5)),
                    1 => format!("d{:016x}", (rng.range(-5, 5) as f64).to_bits()),
                    _ => format!("l{}", rng.range(-5, 5)),
                }
            } else {
                scalar(rng, &mut objs)
            }
        })
        .collect();
    let kinds = ["S", "O", "B", "Q"];
    let mut s = String::new();
    let mut nested = Vec::new();
    for r in 0..3 {
        if r > 0 {
            s += ";";
        }
        let k = *rng.pick(&kinds);
        s += &small_coll(rng, &mut objs, r, k, &pool);
        // Bags hash by the identity of their count cells: never inside another collection
        if k != "B" {
            nested.push(r);
        }
    }
    // a collection of collections for flatten
    s += &format!(";new {} 3", rng.pick(&kinds));
    for r in nested {
        s += &format!(";+ 3 r{r}");
    }
    let mut next = 4usize;
    for _ in 0..rng.range(3, 10) {
        let a = rng.below(4);
        let b = rng.below(3);
        let v = rng.pick(&pool).clone();
        let i = rng.range(-1, 6);
        let j = rng.range(-1, 6);
        let stmt = match rng.below(22) {
            0 => format!("inc {next} {a} {v}"),
            1 => format!("exc {next} {a} {v}"),
            2 => format!("qinc {next} {a} {v}"),
            3 => format!("qexc {next} {a} {v}"),
            4 => format!("uni {next} {a} {b}"),
            5 => format!("int {next} {a} {b}"),
            6 => format!("min {next} {a} {b}"),
            7 => format!("sym {next} {a} {b}"),
            8 => format!("fla {next} {a}"),
            9 => format!("qfla {next} {a}"),
            10 => format!("aset {next} {a}"),
            11 => format!("abag {next} {a}"),
            12 => format!("aseq {next} {a}"),
            13 => format!("aos {next} {a}"),
            14 => format!("app {next} {a} {v}"),
            15 => format!("pre {next} {a} {v}"),
            16 => format!("ins {next} {a} {i} {v}"),
            17 => format!("subs {next} {a} {i} {j}"),
            18 => format!("subo {next} {a} {i} {j}"),
            19 => format!("cp {} {next} {a}", rng.pick(&kinds)),
            20 => format!("uni {next} {a} {next_prev}", next_prev = next - 1),
            _ => format!("aset {next} {}", next - 1),
        };
        s += &format!(";{stmt};p {next}");
        let probes = match rng.below(9) {
            0 => format!("pin {next} {v}"),
            1 => format!("pat {next} {i}"),
            2 => format!("pidx {next} {v}"),
            3 => format!("pcnt {next} {v}"),
            4 => format!("peq {next} {a}"),
            5 => format!("pjeq {next} {a}"),
            6 if numeric => format!("psum {next}"),
            7 if numeric => format!("pmax {next}"),
            8 if numeric => format!("pmin {next}"),
            _ => format!("pjeq {a} {b}"),
        };
        s += &format!(";{probes}");
        next += 1;
    }
    s
}

fn dict_case(rng: &mut Rng) -> String {
    let mut objs = Vec::new();
    let pool = rng.below(5);
    let n = if rng.chance(0.5) { *rng.pick(SIZES) } else { rng.below(60) as usize };
    let keys = values(rng, &mut objs, pool, n);
    let vals = values(rng, &mut objs, 0, n);
    let mut s = String::from("dict 0");
    for (k, v) in keys.iter().zip(vals.iter()) {
        s += &format!(";dput 0 {k} {v}");
        if rng.chance(0.1) {
            s += &format!(";drem 0 {}", rng.pick(&vals));
        }
    }
    s += ";p 0;dcp 1 0;p 1";
    s
}

#[test]
#[ignore]
fn collections() {
    let mut rng = Rng(0x51c0_1c00);
    let n = 1500 * scale();
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..n {
        reqs.push(format!("coll {}", growth_case(&mut rng)));
    }
    check("coll-growth", reqs, 150);
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..n {
        reqs.push(format!("coll {}", collision_case(&mut rng)));
    }
    check("coll-collisions", reqs, 100);
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..(4 * n) {
        reqs.push(format!("coll {}", ops_case(&mut rng)));
    }
    check("coll-ops", reqs, 400);
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..n {
        reqs.push(format!("coll {}", dict_case(&mut rng)));
    }
    check("coll-dict", reqs, 100);
}

#[test]
#[ignore]
fn hashes() {
    let mut rng = Rng(0x4a54);
    let mut objs = Vec::new();
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..20000 * scale() {
        let v = if rng.chance(0.5) { enc_str(&utf16(&mut rng, 24)) } else { scalar(&mut rng, &mut objs) };
        reqs.push(format!("hash {v}"));
    }
    check("hash", reqs, 1000);
}

/// Shrinks a failing collection request (`QVTO_SHRINK=<file with one request>`) by dropping
/// statements while Java and Rust still disagree; prints the result.
#[test]
#[ignore]
fn shrink() {
    let Ok(path) = std::env::var("QVTO_SHRINK") else { return };
    let req = std::fs::read_to_string(path).unwrap().trim().to_string();
    let body = req.strip_prefix("coll ").unwrap();
    let mut stmts: Vec<String> = body.split(';').map(|s| s.to_string()).collect();
    loop {
        let mut cands = Vec::new();
        for i in 0..stmts.len() {
            let mut c = stmts.clone();
            c.remove(i);
            cands.push(format!("coll {}", c.join(";")));
        }
        let answers = Oracle::start().ask_all(&cands);
        let mut found = None;
        for (i, (c, a)) in cands.iter().zip(answers.iter()).enumerate() {
            let got = std::panic::catch_unwind(|| eval(c));
            if !a.contains("EXC") && got.is_ok_and(|g| &g != a) {
                found = Some(i);
                break;
            }
        }
        match found {
            Some(i) => {
                stmts.remove(i);
            }
            None => break,
        }
    }
    let r = format!("coll {}", stmts.join(";"));
    let a = Oracle::start().ask_all(std::slice::from_ref(&r));
    println!("{r}\njava: {}\nrust: {}", a[0], eval(&r));
}

fn hex_ascii(s: &str) -> String {
    common::hex16(s.encode_utf16())
}

#[test]
#[ignore]
fn doubles_to_string() {
    let mut rng = Rng(0xd70a);
    let mut reqs: Vec<String> = Vec::new();
    // powers of ten and their neighbours, both signs
    for e in -325..=309 {
        let p = format!("1e{e}").parse::<f64>().unwrap();
        for b in [p.to_bits().wrapping_sub(1), p.to_bits(), p.to_bits() + 1] {
            reqs.push(format!("dtoa {b:016x}"));
            reqs.push(format!("dtoa {:016x}", b | (1 << 63)));
        }
    }
    // all one- and two-ulp subnormals near the bottom, layout boundaries
    for m in 0..2000u64 {
        reqs.push(format!("dtoa {m:016x}"));
    }
    for x in [1e-3, 1e7, 9999999.999999998, 0.001, 0.0009999999999999998, f64::MAX, f64::MIN_POSITIVE, 2e23, 5e-324] {
        reqs.push(format!("dtoa {:016x}", x.to_bits()));
    }
    for _ in 0..200_000 * scale() {
        reqs.push(format!("dtoa {:016x}", double_bits(&mut rng)));
    }
    for i in -1000..1000 {
        reqs.push(format!("dtoa {:016x}", (i as f64).to_bits()));
        reqs.push(format!("dtoa {:016x}", (i as f64 / 100.0).to_bits()));
    }
    check("dtoa", reqs, 3000);
    let mut reqs: Vec<String> = Vec::new();
    for m in 0..3000u32 {
        reqs.push(format!("ftoa {m:08x}"));
    }
    for _ in 0..100_000 * scale() {
        let b = match rng.below(3) {
            0 => rng.next() as u32,
            1 => (rng.range(-100000, 100000) as f32 / 64.0).to_bits(),
            _ => ((rng.range(1, 99999) as f64) * 10f64.powi(rng.range(-40, 38) as i32)) as f32 as u32,
        };
        reqs.push(format!("ftoa {b:08x}"));
    }
    check("ftoa", reqs, 1000);
}

#[test]
#[ignore]
fn number_parsing() {
    let mut rng = Rng(0x9a25e);
    let mut reqs: Vec<String> = Vec::new();
    let fixed = [
        "1", "1.5", "-1.5", "+2", ".5", "5.", ".", "", " ", " 1 ", "\t2\n", "1e3", "1E-3", "1e", "1e+", "1e+5", "1.5f", "1.5F",
        "1.5d", "1.5D", "1.5ff", "NaN", "-NaN", "+Infinity", "-Infinity", "Infinity ", "infinity", "nan", "0x1p3", "0X1.8P-2",
        "0x.8p1", "0x1.p0", "0x1", "0xp1", "0x1p", "-0x1p-1074", "0x1p-1075", "0x1.8p-1075", "0x0.0000000000001p-1022",
        "0x1.fffffffffffffp1023", "0x1.fffffffffffff8p1023", "0x123456789abcdef0123p0", "1e400", "1e-400", "2.4703282292062328e-324",
        "2.4703282292062327e-324", "4.9e-324", "1_000", "1,5", "１", "٣", "0.000000000000000000000000000000001", "123456789012345678901234567890",
        "9007199254740993", "1.7976931348623157e308", "1.7976931348623159e308", "00012", "-0", "-0.0", "0e0", "1e2147483648", "1e-2147483649",
        "\u{0}1", "1\u{0}", "\u{20}\u{1F}7",
    ];
    for f in fixed {
        for c in ["dparse", "iparse", "lparse"] {
            reqs.push(format!("{c} {}", hex_ascii(f)));
        }
    }
    for _ in 0..60_000 * scale() {
        let s = match rng.below(8) {
            0 => qvto_runtime::java::fmt::double_to_string(f64::from_bits(double_bits(&mut rng))),
            1 => format!("{}", rng.next() as i64),
            2 => format!("{}", rng.next() as i32),
            3 => format!("{}e{}", rng.range(-99999, 99999), rng.range(-330, 330)),
            4 => format!("{}.{}E{}{}", rng.below(1000), rng.below(1_000_000), if rng.chance(0.5) { "-" } else { "" }, rng.below(400)),
            5 => format!("0x{:x}.{:x}p{}", rng.below(1 << 20), rng.next(), rng.range(-1100, 1100)),
            6 => {
                let alphabet = b"0123456789.eE+-xXpPfFdDaN \t";
                (0..rng.range(1, 8)).map(|_| *rng.pick(alphabet) as char).collect()
            }
            _ => format!("{:+}", rng.range(-3_000_000_000, 3_000_000_000)),
        };
        let c = *rng.pick(&["dparse", "dparse", "iparse", "lparse"]);
        reqs.push(format!("{c} {}", hex_ascii(&s)));
    }
    check("parse", reqs, 2000);
}

#[test]
#[ignore]
fn ocl_numbers() {
    let mut rng = Rng(0x0c1);
    let ops = ["+", "-", "*", "/", "div", "mod", "max", "min", "<", ">", "<=", ">=", "=", "<>", "neg", "abs", "floor", "round", "toString"];
    let operand = |rng: &mut Rng| -> String {
        match rng.below(9) {
            0 => format!("i{}", rng.range(-10, 10)),
            1 => format!("i{}", rng.next() as i32),
            2 => format!("i{}", *rng.pick(&[i32::MIN, i32::MAX, -1, 0, 1, 46341, -46341, 65536])),
            3 => format!("l{}", *rng.pick(&[i64::MIN, i64::MAX, 2147483648, -2147483649, 3037000500, 1 << 53, (1 << 53) + 1])),
            4 => {
                let v = rng.next() as i64;
                if v as i32 as i64 == v { format!("i{v}") } else { format!("l{v}") }
            }
            5 => format!("d{:016x}", (rng.range(-20, 20) as f64 / 4.0).to_bits()),
            6 => format!("d{:016x}", rng.pick(&[0f64, -0.0, 0.5, -0.5, 1.5, -2.5, 1e10, 3e9, -3e9, 2147483647.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 9007199254740993.0]).to_bits()),
            _ => format!("d{:016x}", double_bits(rng)),
        }
    };
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..40_000 * scale() {
        let op = *rng.pick(&ops);
        let a = operand(&mut rng);
        if matches!(op, "neg" | "abs" | "floor" | "round" | "toString") {
            reqs.push(format!("num {op} {a}"));
        } else {
            let b = operand(&mut rng);
            reqs.push(format!("num {op} {a} {b}"));
        }
    }
    check("ocl-num", reqs, 2000);
}

#[test]
#[ignore]
fn strings() {
    let mut rng = Rng(0x57);
    let mut reqs: Vec<String> = Vec::new();
    let enc = |u: &[u16]| common::hex16(u.iter().copied());
    let interesting: Vec<Vec<u16>> = ["", "a", "ß", "Σ", "ΟΔΟΣ", "ΣΑ", "Α Σ", "ΑΣ.", "ΑΣ'Α", "İ", "ı", "ﬀ", "ŉ", "ǅ", "\u{1F600}", "Straße", "ÉCOLE", " x ", "\u{0}\u{1}a\u{20}", "\u{a0}x\u{a0}", "aAbB", "Ａｂ", "𐐨𐐀"]
        .iter()
        .map(|s| s.encode_utf16().collect())
        .collect();
    let mut strings: Vec<Vec<u16>> = interesting.clone();
    for _ in 0..3000 {
        strings.push(utf16(&mut rng, 12));
    }
    for s in &strings {
        for op in ["hash", "len", "upper", "lower", "trim", "tok"] {
            reqs.push(format!("str {op} {}", enc(s)));
        }
    }
    for _ in 0..30_000 * scale() {
        let a = rng.pick(&strings).clone();
        // b: often a piece of a
        let b: Vec<u16> = if rng.chance(0.6) && !a.is_empty() {
            let i = rng.below(a.len() as u64) as usize;
            let j = i + rng.below((a.len() - i) as u64 + 1) as usize;
            a[i..j].to_vec()
        } else {
            rng.pick(&strings).clone()
        };
        let c = rng.pick(&strings).clone();
        let req = match rng.below(9) {
            0 => format!("str cmp {} {}", enc(&a), enc(&b)),
            1 => format!("str eqic {} {}", enc(&a), enc(&if rng.chance(0.5) { b.clone() } else { a.iter().map(|&u| if rng.chance(0.5) { qvto_runtime::java::chars::to_upper_unit(u) } else { u }).collect() })),
            2 => format!("str idx {} {}", enc(&a), enc(&b)),
            3 => format!("str lidx {} {}", enc(&a), enc(&b)),
            4 => format!("str repl {} {} {}", enc(&a), enc(&b), enc(&c)),
            5 => format!("str sw {} {}", enc(&a), enc(&b)),
            6 => format!("str ew {} {}", enc(&a), enc(&b)),
            7 => {
                let n = a.len() as i64;
                format!("str sub {} {} {}", enc(&a), rng.range(-1, n + 1), rng.range(-1, n + 1))
            }
            _ => format!("str tok {} {}", enc(&a), enc(&b)),
        };
        reqs.push(req);
    }
    check("string", reqs, 2000);
}

#[test]
#[ignore]
fn characters() {
    let reqs: Vec<String> = (0..=0xffffu32).map(|u| format!("char {u}")).collect();
    check("char", reqs, 600);
    let mut rng = Rng(0xc9);
    let mut reqs: Vec<String> = (0..0x2_0000u32).filter(|c| !(0xd800..0xe000).contains(c)).map(|c| format!("cp {c}")).collect();
    for _ in 0..20_000 {
        reqs.push(format!("cp {}", rng.range(0x20000, 0x10ffff)));
    }
    check("codepoint", reqs, 600);
}

#[test]
#[ignore]
fn to_string() {
    let mut rng = Rng(0x7057);
    let mut reqs: Vec<String> = Vec::new();
    for _ in 0..5000 * scale() {
        let mut objs = Vec::new();
        let kind = *rng.pick(&["S", "O", "B", "Q"]);
        let mut s = format!("new {kind} 0");
        for _ in 0..rng.below(12) {
            let v = loop {
                let v = scalar(&mut rng, &mut objs);
                // objects print their Java class; strings with lone surrogates cannot be UTF-8
                if !v.starts_with('o') && !v.starts_with('s') {
                    break v;
                }
                if v.starts_with('s') {
                    let u = common::unhex16(&v[1..]);
                    if char::decode_utf16(u.iter().copied()).all(|r| r.is_ok()) {
                        break v;
                    }
                }
            };
            s += &format!(";+ 0 {v}");
        }
        s += ";ps 0;dict 1";
        for _ in 0..rng.below(5) {
            s += &format!(";dput 1 i{} d{:016x}", rng.range(-50, 50), (rng.range(-8, 8) as f64 / 8.0).to_bits());
        }
        s += ";ps 1";
        reqs.push(format!("coll {s}"));
    }
    check("tostring", reqs, 300);
}
