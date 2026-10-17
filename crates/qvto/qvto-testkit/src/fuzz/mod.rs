//! Generators for differential fuzzing against the Java reference runner (PLAN Q7):
//! random well-typed OCL/ImperativeOCL expressions ([`expr`]), random small transformations
//! ([`prog`]) and random input models of a small metamodel (`fuzz.ecore`, [`model`]).
//!
//! Generated cases are ordinary case files (`case.rs`) run by both sides; `qvto fuzz` drives the
//! campaigns (generate a batch, run the Java runner once on it, run the engine, compare,
//! minimise what differs).

pub mod expr;
pub mod model;
pub mod prog;

/// The fuzz metamodel (`fz`, `http://qvto-rs/fuzz`): an enumeration, an abstract named class,
/// a root with containments, nodes with attributes of every primitive box kind, many-valued
/// attributes (unique and not), a containment with an opposite, ordered and unordered
/// cross-references, a subclass, and items with nullable and float attributes.
pub const FUZZ_ECORE: &str = include_str!("fuzz.ecore");
pub const FUZZ_NS_URI: &str = "http://qvto-rs/fuzz";

/// splitmix64: small, fast, seedable; the generators need nothing better.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i64
    }
    /// True with probability `p` percent.
    pub fn pct(&mut self, p: u32) -> bool {
        (self.next_u64() % 100) < p as u64
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    /// An index chosen by weight.
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u32 = weights.iter().sum();
        let mut r = (self.next_u64() % total.max(1) as u64) as u32;
        for (i, w) in weights.iter().enumerate() {
            if r < *w {
                return i;
            }
            r -= w;
        }
        weights.len() - 1
    }
    /// A derived generator (for independent streams, e.g. one per case).
    pub fn fork(&mut self) -> Rng {
        Rng::new(self.next_u64())
    }
}

/// Quotes a string as a QVTo string literal (`'...'` with Eclipse's escapes).
pub fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('\'');
    for c in s.chars() {
        match c {
            '\'' => o.push_str("\\'"),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o.push('\'');
    o
}
