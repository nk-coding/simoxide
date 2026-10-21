//! Digests of outputs that reproduce third-party models in full: the canonical dumps of the
//! EMF oracle (`reference/oracles/pcm-model/golden*/`) and the Java runner's output dumps of
//! transformations on them (`crates/qvto/qvto-testkit/fixtures`, the adapter corpus of
//! `simoxide-reconf`). The repository keeps their SHA-256 and length, not the text
//! (`third_party/README.md`, "Dumps"): the tests produce the dump from the submodules and compare
//! digests; the Java oracles write the full dumps (git-ignored) when they run.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// SHA-256 (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*c);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 and length of a text that is not committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digest {
    pub sha256: String,
    pub bytes: usize,
}

impl Digest {
    pub fn of(text: &str) -> Digest {
        Digest {
            sha256: sha256_hex(text.as_bytes()),
            bytes: text.len(),
        }
    }

    pub fn matches(&self, text: &str) -> bool {
        text.len() == self.bytes && sha256_hex(text.as_bytes()) == self.sha256
    }

    /// `sha256<TAB>hex` and `bytes<TAB>n` lines.
    pub fn to_text(&self) -> String {
        format!("sha256\t{}\nbytes\t{}\n", self.sha256, self.bytes)
    }

    /// Reads the `sha256` and `bytes` lines of a text (other lines are ignored).
    pub fn parse(text: &str) -> Option<Digest> {
        let field = |k: &str| {
            text.lines().find_map(|l| {
                let (a, b) = l.split_once('\t')?;
                (a == k).then_some(b)
            })
        };
        let sha256 = field("sha256")?;
        if sha256.len() != 64 || !sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(Digest {
            sha256: sha256.to_ascii_lowercase(),
            bytes: field("bytes")?.parse().ok()?,
        })
    }
}

/// Name prefixes of the dumps in `reference/oracles/pcm-model/golden*/` that reproduce
/// third-party models (`models.txt` / `spd-models.txt` names): the three submodule repositories
/// themselves, the corpus directories that link into them, and Slingshot's minimal example.
pub const THIRD_PARTY_DUMPS: &[&str] = &[
    "Palladio-Example-Models_",
    "Palladio-Analyzer-Slingshot-E2E-Tests_",
    "Palladio-Documentation-Slingshot_",
    "corpus_x_pem_",
    "corpus_x_ss_",
    "slingshot_spd_",
];

/// Whether the dump `name` (without `.jsonl`) is one of a third-party model.
pub fn is_third_party_dump(name: &str) -> bool {
    THIRD_PARTY_DUMPS.iter().any(|p| name.starts_with(p))
}

/// The digest list of a dump directory: `name<TAB>header<TAB>sha256<TAB>bytes` per dump, the
/// digest over the dump without its header line (the first line, e.g. `{"emf_errors":0}`).
pub const DIGESTS: &str = "third-party.digests";

const DIGESTS_HEADER: &str = "# Canonical dumps of third-party models (third_party/README.md, \"Dumps\"): the dumps are not\n\
# committed, only their digests. name<TAB>header line<TAB>sha256 of the rest<TAB>its length in bytes.\n\
# Written by `simoxide-thirdparty digests DIR` from the dumps the oracle wrote (run.sh, run-spd.sh).\n";

/// A dump's header line and the digest of the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpDigest {
    pub header: String,
    pub digest: Digest,
}

/// Reads `DIR/third-party.digests` (empty if there is none).
pub fn read_digests(dir: &Path) -> io::Result<BTreeMap<String, DumpDigest>> {
    let text = match fs::read_to_string(dir.join(DIGESTS)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    let mut out = BTreeMap::new();
    for (i, l) in text.lines().enumerate() {
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = l.split('\t').collect();
        let bad = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: expected name, header, sha256, bytes",
                    DIGESTS,
                    i + 1
                ),
            )
        };
        if f.len() != 4 {
            return Err(bad());
        }
        let digest =
            Digest::parse(&format!("sha256\t{}\nbytes\t{}\n", f[2], f[3])).ok_or_else(bad)?;
        out.insert(
            f[0].to_string(),
            DumpDigest {
                header: f[1].to_string(),
                digest,
            },
        );
    }
    Ok(out)
}

/// Splits a dump into its header line and the rest.
pub fn split_dump(text: &str) -> (&str, &str) {
    text.split_once('\n').unwrap_or((text, ""))
}

/// Writes `DIR/third-party.digests` from the third-party dumps `DIR/<name>.jsonl`
/// ([`is_third_party_dump`]); a dump that is absent keeps its entry. Returns the number of dumps
/// read.
pub fn write_digests(dir: &Path) -> io::Result<usize> {
    let mut all = read_digests(dir)?;
    let mut n = 0;
    let mut files: Vec<_> = fs::read_dir(dir)?.flatten().map(|e| e.path()).collect();
    files.sort();
    for p in files {
        let Some(name) = p
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if !is_third_party_dump(name) {
            continue;
        }
        let text = fs::read_to_string(&p)?;
        let (header, body) = split_dump(&text);
        all.insert(
            name.to_string(),
            DumpDigest {
                header: header.to_string(),
                digest: Digest::of(body),
            },
        );
        n += 1;
    }
    let mut out = String::from(DIGESTS_HEADER);
    for (name, d) in &all {
        out.push_str(&format!(
            "{name}\t{}\t{}\t{}\n",
            d.header, d.digest.sha256, d.digest.bytes
        ));
    }
    fs::write(dir.join(DIGESTS), out)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_values() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // two blocks
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let a = "a".repeat(1000);
        assert_eq!(
            sha256_hex(a.as_bytes()),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn digest_text_round_trip() {
        let d = Digest::of("x\n");
        assert_eq!(Digest::parse(&d.to_text()), Some(d.clone()));
        assert!(d.matches("x\n"));
        assert!(!d.matches("x"));
        assert_eq!(Digest::parse("sha256\tzz\nbytes\t1\n"), None);
    }
}
