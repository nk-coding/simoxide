//! Deterministic token-level mutants of corpus files, for differential testing against
//! Eclipse's parser (`tests/mutants.rs`, `examples/mutants.rs`).

use qvto_syntax::TokenKind;

/// Replacement tokens for the "replace" mutation.
const REPLACEMENTS: &[&str] = &[
    ";", "(", ")", "{", "}", "[", "]", ".", "->", ":=", "=", ",", "|", ":", "::", "if", "then", "else", "endif",
    "var", "x", "1", "'s'", "not", "-", "*", "and", "in", "let", "return", "object", "map", "when", "end", "init",
];

pub const PER_FILE: u64 = 3;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A mutant: description and text.
pub struct Mutant {
    pub id: String,
    pub what: String,
    pub text: String,
}

/// The mutants of one file (`key` is the fixture key, which seeds the generator).
pub fn mutants(key: &str, src: &str) -> Vec<Mutant> {
    let toks: Vec<_> = qvto_syntax::lexer::lex(src).tokens.into_iter().filter(|t| t.kind != TokenKind::Eof).collect();
    let mut out = Vec::new();
    if toks.len() < 2 {
        return out;
    }
    let mut st = fnv(key);
    for n in 0..PER_FILE {
        let r = splitmix(&mut st);
        let i = (splitmix(&mut st) % (toks.len() as u64 - 1)) as usize;
        let t = toks[i];
        let (s, e) = (t.span.start as usize, t.span.end as usize);
        let tok = &src[s..e];
        let (what, text) = match r % 4 {
            0 => (format!("delete {s} {tok:?}"), format!("{} {}", &src[..s], &src[e..])),
            1 => (format!("duplicate {s} {tok:?}"), format!("{} {} {}", &src[..e], tok, &src[e..])),
            2 => {
                let u = toks[i + 1];
                let (s2, e2) = (u.span.start as usize, u.span.end as usize);
                (
                    format!("swap {s} {tok:?} {:?}", &src[s2..e2]),
                    format!("{} {} {} {} {}", &src[..s], &src[s2..e2], &src[e..s2], tok, &src[e2..]),
                )
            }
            _ => {
                let rep = REPLACEMENTS[(splitmix(&mut st) % REPLACEMENTS.len() as u64) as usize];
                (format!("replace {s} {tok:?} {rep:?}"), format!("{} {rep} {}", &src[..s], &src[e..]))
            }
        };
        out.push(Mutant { id: format!("{key}#{n}"), what, text });
    }
    out
}
