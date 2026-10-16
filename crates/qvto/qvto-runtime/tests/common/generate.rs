//! Deterministic generators of oracle requests.

use super::hex16;

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/// A random UTF-16 string: ASCII, Latin-1, Greek, CJK, supplementary pairs and (rarely) lone
/// surrogates.
pub fn utf16(rng: &mut Rng, max_len: usize) -> Vec<u16> {
    let n = rng.below(max_len as u64 + 1) as usize;
    let mut v = Vec::with_capacity(n);
    let profile = rng.below(4);
    while v.len() < n {
        let k = if profile == 0 { 0 } else { rng.below(12) };
        match k {
            0..=5 => v.push(rng.range(0x20, 0x7e) as u16),
            6 => v.push(rng.range(0xa0, 0x17f) as u16),
            7 => v.push(rng.range(0x370, 0x3ff) as u16),
            8 => v.push(rng.range(0x4e00, 0x9fff) as u16),
            9 => {
                let cp = rng.range(0x10000, 0x10ffff) as u32 - 0x10000;
                v.push(0xd800 | (cp >> 10) as u16);
                v.push(0xdc00 | (cp & 0x3ff) as u16);
            }
            10 => v.push(rng.range(0, 0xffff) as u16),
            _ => {
                if rng.chance(0.2) {
                    v.push(rng.range(0xd800, 0xdfff) as u16)
                } else {
                    v.push(rng.range(0x20, 0x7e) as u16)
                }
            }
        }
    }
    v
}

pub fn enc_str(units: &[u16]) -> String {
    format!("s{}", hex16(units.iter().copied()))
}

/// Strings with equal `hashCode`: concatenations of `Aa` and `BB` (both hash to 2112).
pub fn colliding_strings(rng: &mut Rng, blocks: u32, count: usize) -> Vec<String> {
    let prefix: Vec<u16> = (0..rng.below(3))
        .map(|_| rng.range(0x61, 0x7a) as u16)
        .collect();
    let mut idx: Vec<u64> = (0..(1u64 << blocks)).collect();
    // shuffle
    for i in (1..idx.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        idx.swap(i, j);
    }
    idx.truncate(count);
    idx.iter()
        .map(|&bits| {
            let mut u = prefix.clone();
            for b in 0..blocks {
                if bits >> b & 1 == 1 {
                    u.extend_from_slice(&[0x41, 0x61]);
                } else {
                    u.extend_from_slice(&[0x42, 0x42]);
                }
            }
            enc_str(&u)
        })
        .collect()
}

pub fn double_bits(rng: &mut Rng) -> u64 {
    match rng.below(10) {
        0 => *rng.pick(&[
            0u64,
            0x8000_0000_0000_0000,
            0x7ff8_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
            1,
            0x000f_ffff_ffff_ffff,
            0x0010_0000_0000_0000,
            0x7fef_ffff_ffff_ffff,
        ]),
        1 => (rng.range(-1000, 1000) as f64).to_bits(),
        2 => (rng.range(-1000, 1000) as f64 / 8.0).to_bits(),
        3 => rng.below(0x0010_0000_0000_0000) | (rng.below(2) << 63),
        4 => {
            let e = rng.range(-30, 30) as i32;
            (10f64.powi(e) * if rng.chance(0.5) { 1.0 } else { -1.0 }).to_bits()
        }
        5 => (rng.range(1, 999999) as f64 * 10f64.powi(rng.range(-12, 12) as i32)).to_bits(),
        _ => rng.next(),
    }
}

/// A random scalar value in the oracle encoding.
pub fn scalar(rng: &mut Rng, objs: &mut Vec<(u32, i32)>) -> String {
    match rng.below(20) {
        0 => "n".into(),
        1..=4 => format!("i{}", rng.range(-40, 40)),
        5 => format!("i{}", rng.next() as i32),
        6 => format!("l{}", rng.next() as i64),
        7 => format!("l{}", rng.range(-40, 40)),
        8 | 9 => format!("d{:016x}", double_bits(rng)),
        10 => format!("d{:016x}", (rng.range(-40, 40) as f64).to_bits()),
        11 => format!("b{}", rng.below(2)),
        12 => format!("h{}", rng.range(-40, 40)),
        13 => format!("y{}", rng.range(-40, 40)),
        14 => format!("c{:04x}", rng.range(0x30, 0x7a)),
        15 => format!("f{:08x}", (rng.range(-20, 20) as f32 / 4.0).to_bits()),
        16 => obj(rng, objs),
        _ => enc_str(&utf16(rng, 6)),
    }
}

/// An object with a fixed identity hash (reused with some probability).
pub fn obj(rng: &mut Rng, objs: &mut Vec<(u32, i32)>) -> String {
    if !objs.is_empty() && rng.chance(0.3) {
        let (id, h) = *rng.pick(objs);
        return format!("o{id}.{h}");
    }
    let id = objs.len() as u32 + 1;
    let h = (rng.next() as u32 & 0x7fff_ffff) as i32;
    objs.push((id, h));
    format!("o{id}.{h}")
}
