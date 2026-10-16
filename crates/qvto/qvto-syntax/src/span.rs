//! Byte spans and line/column mapping.

use std::fmt;

/// A half-open byte range `start..end` in the source text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[inline]
    pub const fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    /// An empty span at `pos`.
    #[inline]
    pub const fn at(pos: u32) -> Span {
        Span {
            start: pos,
            end: pos,
        }
    }

    /// The smallest span covering both.
    #[inline]
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    #[inline]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    #[inline]
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The text this span covers.
    #[inline]
    pub fn text(self, src: &str) -> &str {
        &src[self.range()]
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A 1-based line and column. Columns count characters (UTF-16 code units, as Eclipse does),
/// with a tab counting as one character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// Line starts of a source text. Line breaks are LF, CR LF and a lone CR (Eclipse rewrites a lone
/// CR to LF before lexing, so it counts as a line break for line numbers in diagnostics, logs and
/// stack traces).
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> LineIndex {
        let b = src.as_bytes();
        let mut starts = vec![0u32];
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\n' => starts.push(i as u32 + 1),
                b'\r' => {
                    if b.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    starts.push(i as u32 + 1);
                }
                _ => {}
            }
            i += 1;
        }
        LineIndex { starts }
    }

    /// The 1-based line of a byte offset.
    pub fn line(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    /// The 1-based line and column of a byte offset.
    pub fn line_col(&self, src: &str, offset: u32) -> LineCol {
        let line = self.line(offset);
        let start = self.starts[line as usize - 1] as usize;
        let col = utf16_len(&src[start..offset as usize]) as u32 + 1;
        LineCol { line, col }
    }

    /// Byte offset of the start of a 1-based line.
    pub fn line_start(&self, line: u32) -> Option<u32> {
        self.starts.get(line.checked_sub(1)? as usize).copied()
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
}

/// Number of UTF-16 code units of a string.
pub fn utf16_len(s: &str) -> usize {
    if s.is_ascii() {
        return s.len();
    }
    s.chars().map(char::len_utf16).sum()
}

/// Converts a UTF-16 offset (as Eclipse reports them) into a byte offset of `src`.
pub fn utf16_to_byte(src: &str, utf16: usize) -> usize {
    if src.is_ascii() {
        return utf16.min(src.len());
    }
    let mut u = 0;
    for (i, c) in src.char_indices() {
        if u >= utf16 {
            return i;
        }
        u += c.len_utf16();
    }
    src.len()
}

/// Converts a byte offset of `src` into a UTF-16 offset.
pub fn byte_to_utf16(src: &str, byte: usize) -> usize {
    utf16_len(&src[..byte.min(src.len())])
}
