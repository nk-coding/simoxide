//! `Str`: an immutable, reference-counted Java string.
//!
//! One allocation: a header (reference count, Java hash, UTF-16 length, byte length) followed by
//! the text. The text is WTF-8: UTF-8 that may also hold lone surrogates (U+D800–U+DFFF encoded
//! like other three-byte code points), because Java strings are UTF-16 and OCL's `substring`,
//! `at` and `characters` can split a surrogate pair. Such strings are rare; [`Str::as_str`]
//! returns `None` for them and [`Str::to_str_lossy`] replaces lone surrogates by U+FFFD.
//!
//! The Java hash (`String.hashCode`) and the UTF-16 length are computed once, at creation.
//! `Str` is 8 bytes and, like `Rc`, neither `Send` nor `Sync`.

use crate::java::hash::utf16_hash;
use std::alloc::{self, Layout};
use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

#[repr(C)]
struct Header {
    rc: Cell<usize>,
    hash: i32,
    len16: u32,
    len: u32,
    /// bit 0: has lone surrogates
    flags: u32,
}

const HAS_SURROGATES: u32 = 1;

/// An immutable Java string (see the module docs).
pub struct Str {
    ptr: NonNull<Header>,
    _not_send: PhantomData<*const ()>,
}

fn layout(len: usize) -> Layout {
    Layout::from_size_align(std::mem::size_of::<Header>() + len, std::mem::align_of::<Header>())
        .expect("string too large")
}

impl Str {
    /// From WTF-8 bytes. The caller guarantees well-formed WTF-8 (no surrogate pair encoded as
    /// two three-byte sequences).
    fn from_wtf8_parts(bytes: &[u8], hash: i32, len16: usize, surrogates: bool) -> Str {
        assert!(bytes.len() <= u32::MAX as usize && len16 <= u32::MAX as usize, "string too large");
        unsafe {
            let lay = layout(bytes.len());
            let p = alloc::alloc(lay) as *mut Header;
            let Some(ptr) = NonNull::new(p) else { alloc::handle_alloc_error(lay) };
            p.write(Header {
                rc: Cell::new(1),
                hash,
                len16: len16 as u32,
                len: bytes.len() as u32,
                flags: if surrogates { HAS_SURROGATES } else { 0 },
            });
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (p as *mut u8).add(std::mem::size_of::<Header>()), bytes.len());
            Str { ptr, _not_send: PhantomData }
        }
    }

    /// A string from Rust text.
    pub fn new(s: &str) -> Str {
        if s.is_ascii() {
            let mut h: i32 = 0;
            for &b in s.as_bytes() {
                h = h.wrapping_mul(31).wrapping_add(i32::from(b));
            }
            Self::from_wtf8_parts(s.as_bytes(), h, s.len(), false)
        } else {
            let mut h: i32 = 0;
            let mut n = 0usize;
            for u in s.encode_utf16() {
                h = h.wrapping_mul(31).wrapping_add(i32::from(u));
                n += 1;
            }
            Self::from_wtf8_parts(s.as_bytes(), h, n, false)
        }
    }

    /// A string from UTF-16 code units (lone surrogates allowed).
    pub fn from_utf16(units: &[u16]) -> Str {
        let mut bytes = Vec::with_capacity(units.len());
        let mut surrogates = false;
        let mut i = 0;
        while i < units.len() {
            let u = units[i];
            let cp = if (0xD800..0xDC00).contains(&u) && i + 1 < units.len() && (0xDC00..0xE000).contains(&units[i + 1]) {
                i += 1;
                0x10000 + (((u as u32) - 0xD800) << 10) + ((units[i] as u32) - 0xDC00)
            } else {
                if (0xD800..0xE000).contains(&u) {
                    surrogates = true;
                }
                u as u32
            };
            push_wtf8(&mut bytes, cp);
            i += 1;
        }
        Self::from_wtf8_parts(&bytes, utf16_hash(units.iter().copied()), units.len(), surrogates)
    }

    #[inline]
    fn header(&self) -> &Header {
        unsafe { self.ptr.as_ref() }
    }

    /// The WTF-8 bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        let h = self.header();
        unsafe {
            std::slice::from_raw_parts((self.ptr.as_ptr() as *const u8).add(std::mem::size_of::<Header>()), h.len as usize)
        }
    }

    /// The text, unless it contains lone surrogates.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        if self.has_lone_surrogates() {
            None
        } else {
            Some(unsafe { std::str::from_utf8_unchecked(self.as_bytes()) })
        }
    }

    /// The text with lone surrogates replaced by U+FFFD.
    pub fn to_str_lossy(&self) -> Cow<'_, str> {
        match self.as_str() {
            Some(s) => Cow::Borrowed(s),
            None => Cow::Owned(char::decode_utf16(self.units()).map(|r| r.unwrap_or('\u{FFFD}')).collect()),
        }
    }

    /// `String.hashCode()`.
    #[inline]
    pub fn java_hash(&self) -> i32 {
        self.header().hash
    }

    /// `String.length()`: the number of UTF-16 code units.
    #[inline]
    pub fn len16(&self) -> usize {
        self.header().len16 as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.header().len == 0
    }

    /// All characters are ASCII (then UTF-16 indices are byte indices).
    #[inline]
    pub fn is_ascii(&self) -> bool {
        let h = self.header();
        h.len == h.len16
    }

    #[inline]
    pub fn has_lone_surrogates(&self) -> bool {
        self.header().flags & HAS_SURROGATES != 0
    }

    /// The UTF-16 code units.
    pub fn units(&self) -> Units<'_> {
        Units { bytes: self.as_bytes(), pos: 0, low: None }
    }

    pub fn to_utf16(&self) -> Vec<u16> {
        let mut v = Vec::with_capacity(self.len16());
        v.extend(self.units());
        v
    }

    /// Code points (lone surrogates as themselves).
    pub fn code_points(&self) -> CodePoints<'_> {
        CodePoints { bytes: self.as_bytes(), pos: 0 }
    }

    /// `this + other` (`String.concat`).
    pub fn concat(&self, other: &Str) -> Str {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        if self.has_lone_surrogates() || other.has_lone_surrogates() {
            let mut u = self.to_utf16();
            u.extend(other.units());
            return Str::from_utf16(&u);
        }
        let mut b = Vec::with_capacity(self.as_bytes().len() + other.as_bytes().len());
        b.extend_from_slice(self.as_bytes());
        b.extend_from_slice(other.as_bytes());
        // hash(a + b) = hash(a) * 31^len16(b) + hash(b)
        let mut p: i32 = 1;
        let mut base: i32 = 31;
        let mut e = other.len16();
        while e > 0 {
            if e & 1 == 1 {
                p = p.wrapping_mul(base);
            }
            base = base.wrapping_mul(base);
            e >>= 1;
        }
        let h = self.java_hash().wrapping_mul(p).wrapping_add(other.java_hash());
        Self::from_wtf8_parts(&b, h, self.len16() + other.len16(), false)
    }

    /// True if both point to the same allocation.
    #[inline]
    pub fn ptr_eq(a: &Str, b: &Str) -> bool {
        a.ptr == b.ptr
    }
}

fn push_wtf8(out: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
}

#[inline]
fn decode_at(bytes: &[u8], pos: usize) -> (u32, usize) {
    let b0 = bytes[pos] as u32;
    if b0 < 0x80 {
        (b0, 1)
    } else if b0 < 0xE0 {
        (((b0 & 0x1F) << 6) | (bytes[pos + 1] as u32 & 0x3F), 2)
    } else if b0 < 0xF0 {
        (((b0 & 0x0F) << 12) | ((bytes[pos + 1] as u32 & 0x3F) << 6) | (bytes[pos + 2] as u32 & 0x3F), 3)
    } else {
        (
            ((b0 & 0x07) << 18)
                | ((bytes[pos + 1] as u32 & 0x3F) << 12)
                | ((bytes[pos + 2] as u32 & 0x3F) << 6)
                | (bytes[pos + 3] as u32 & 0x3F),
            4,
        )
    }
}

/// Iterator over UTF-16 code units.
pub struct Units<'a> {
    bytes: &'a [u8],
    pos: usize,
    low: Option<u16>,
}

impl Iterator for Units<'_> {
    type Item = u16;
    #[inline]
    fn next(&mut self) -> Option<u16> {
        if let Some(l) = self.low.take() {
            return Some(l);
        }
        if self.pos >= self.bytes.len() {
            return None;
        }
        let (cp, n) = decode_at(self.bytes, self.pos);
        self.pos += n;
        if cp >= 0x10000 {
            let c = cp - 0x10000;
            self.low = Some(0xDC00 | (c & 0x3FF) as u16);
            Some(0xD800 | (c >> 10) as u16)
        } else {
            Some(cp as u16)
        }
    }
}

/// Iterator over code points.
pub struct CodePoints<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Iterator for CodePoints<'_> {
    type Item = u32;
    #[inline]
    fn next(&mut self) -> Option<u32> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let (cp, n) = decode_at(self.bytes, self.pos);
        self.pos += n;
        Some(cp)
    }
}

impl Clone for Str {
    #[inline]
    fn clone(&self) -> Str {
        let h = self.header();
        h.rc.set(h.rc.get() + 1);
        Str { ptr: self.ptr, _not_send: PhantomData }
    }
}

impl Drop for Str {
    #[inline]
    fn drop(&mut self) {
        let h = self.header();
        let rc = h.rc.get() - 1;
        if rc == 0 {
            let len = h.len as usize;
            unsafe { alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout(len)) }
        } else {
            h.rc.set(rc);
        }
    }
}

impl PartialEq for Str {
    #[inline]
    fn eq(&self, other: &Str) -> bool {
        Str::ptr_eq(self, other) || (self.java_hash() == other.java_hash() && self.as_bytes() == other.as_bytes())
    }
}
impl Eq for Str {}

impl std::hash::Hash for Str {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_i32(self.java_hash());
    }
}

impl From<&str> for Str {
    fn from(s: &str) -> Str {
        Str::new(s)
    }
}

impl fmt::Debug for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.to_str_lossy(), f)
    }
}

impl fmt::Display for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java::hash::str_hash;

    #[test]
    fn basics() {
        let s = Str::new("héllo \u{1F600}");
        assert_eq!(s.java_hash(), str_hash("héllo \u{1F600}"));
        assert_eq!(s.len16(), 9);
        assert_eq!(s.as_str(), Some("héllo \u{1F600}"));
        assert!(!s.is_ascii());
        let t = s.clone();
        assert!(Str::ptr_eq(&s, &t));
        assert_eq!(s.to_utf16(), "héllo \u{1F600}".encode_utf16().collect::<Vec<_>>());
    }

    #[test]
    fn lone_surrogates_and_concat() {
        let hi = Str::from_utf16(&[0x61, 0xD83D]);
        let lo = Str::from_utf16(&[0xDE00, 0x62]);
        assert!(hi.has_lone_surrogates() && lo.has_lone_surrogates());
        assert_eq!(hi.as_str(), None);
        let j = hi.concat(&lo);
        assert!(!j.has_lone_surrogates());
        assert_eq!(j.as_str(), Some("a\u{1F600}b"));
        assert_eq!(j.java_hash(), str_hash("a\u{1F600}b"));
        let a = Str::new("abc");
        let b = Str::new("déf");
        assert_eq!(a.concat(&b).java_hash(), str_hash("abcdéf"));
        assert_eq!(a.concat(&b).len16(), 6);
        assert_eq!(std::mem::size_of::<Str>(), 8);
    }
}
