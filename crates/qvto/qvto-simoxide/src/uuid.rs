//! New object ids in the format of EMF's `EcoreUtil.generateUUID()` (`_` and 22 characters of
//! EMF's base64 alphabet over 16 bytes), from a seeded generator so that runs are reproducible.
//! PCM's generated `IdentifierImpl` sets such an id in its constructor; dynamic EMF (the Java
//! reference runner of the corpus) does not.

const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// How the store assigns `Identifier.id` to new objects.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum IdPolicy {
    /// No id (dynamic EMF; equal to the corpus's Java results).
    #[default]
    None,
    /// An EMF-style UUID from a generator seeded with `seed` (generated PCM code, reproducible).
    Uuid { seed: u64 },
}

#[derive(Clone, Debug)]
pub(crate) struct UuidGen(u64);

impl UuidGen {
    pub fn new(seed: u64) -> UuidGen {
        UuidGen(seed)
    }

    /// The generator's state (a generator in this state is `UuidGen::new(state)`).
    pub fn state(&self) -> u64 {
        self.0
    }

    fn next_u64(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next(&mut self) -> String {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&self.next_u64().to_be_bytes());
        b[8..].copy_from_slice(&self.next_u64().to_be_bytes());
        encode(&b)
    }
}

/// `EcoreUtil.generateUUID`'s encoding of the 16 bytes of a UUID.
pub fn encode(u: &[u8; 16]) -> String {
    let d = |i: u8| DIGITS[(i & 0x3F) as usize] as char;
    let mut s = String::with_capacity(23);
    s.push('_');
    for i in 0..5 {
        let (a, b, c) = (u[3 * i], u[3 * i + 1], u[3 * i + 2]);
        s.push(d(a >> 2));
        s.push(d(((a << 4) & 0x30) | ((b >> 4) & 0x0F)));
        s.push(d(((b << 2) & 0x3C) | ((c >> 6) & 0x03)));
        s.push(d(c));
    }
    s.push(d(u[15] >> 2));
    s.push(d((u[15] << 4) & 0x30));
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn format() {
        let mut g = super::UuidGen::new(7);
        let a = g.next();
        assert_eq!(a.len(), 23);
        assert!(a.starts_with('_'));
        assert_ne!(a, g.next());
        assert_eq!(super::UuidGen::new(7).next(), a);
    }
}
