//! A port of the parts of EMF's `org.eclipse.emf.common.util.URI` that model loading and saving
//! depend on: parsing into scheme, authority, device, segments, query and fragment; `resolve`;
//! `deresolve` (relative `href`s in saved XMI); file URIs.
//!
//! Ported from EMF (`URI.java`, EPL-2.0); see `LICENSES`.

use std::fmt;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Uri {
    scheme: Option<String>,
    /// `false` for opaque URIs such as `urn:x` or `mailto:a@b` (the rest is in `authority`).
    hierarchical: bool,
    authority: Option<String>,
    device: Option<String>,
    absolute_path: bool,
    segments: Vec<String>,
    query: Option<String>,
    fragment: Option<String>,
}

const ARCHIVE_SCHEMES: [&str; 3] = ["jar", "zip", "archive"];

impl Uri {
    /// `URI.createURI(s)`: the fragment starts at the first `#`.
    pub fn parse(s: &str) -> Uri {
        let (main, fragment) = match s.find('#') {
            Some(i) => (&s[..i], Some(s[i + 1..].to_string())),
            None => (s, None),
        };
        let mut u = parse_without_fragment(main);
        u.fragment = fragment;
        u
    }

    /// `URI.createFileURI(path)` for an absolute or relative file system path.
    pub fn file(path: &str) -> Uri {
        let encoded = encode_path(path);
        if path.starts_with('/') {
            Uri::parse(&format!("file:{encoded}"))
        } else {
            Uri::parse(&encoded)
        }
    }

    pub fn scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }
    pub fn is_relative(&self) -> bool {
        self.scheme.is_none()
    }
    pub fn is_hierarchical(&self) -> bool {
        self.hierarchical
    }
    pub fn authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }
    pub fn has_fragment(&self) -> bool {
        self.fragment.is_some()
    }
    pub fn has_absolute_path(&self) -> bool {
        self.hierarchical && self.absolute_path
    }
    /// A relative path: no scheme, authority, device and no leading `/`.
    pub fn has_relative_path(&self) -> bool {
        self.hierarchical
            && !self.absolute_path
            && self.authority.is_none()
            && self.device.is_none()
    }
    fn has_path(&self) -> bool {
        self.has_absolute_path() || (self.has_relative_path() && !self.segments.is_empty())
    }
    fn has_empty_path(&self) -> bool {
        self.has_relative_path() && self.segments.is_empty()
    }
    fn has_device_or_path(&self) -> bool {
        self.has_path() || self.device.is_some()
    }
    fn is_archive(&self) -> bool {
        self.scheme
            .as_deref()
            .is_some_and(|s| ARCHIVE_SCHEMES.iter().any(|a| a.eq_ignore_ascii_case(s)))
    }
    pub fn is_file(&self) -> bool {
        self.hierarchical
            && match &self.scheme {
                Some(s) => s.eq_ignore_ascii_case("file"),
                None => self.query.is_none() && !self.absolute_path,
            }
    }
    /// `platform:/resource/...` or `platform:/plugin/...` with at least two segments.
    pub fn is_platform(&self) -> bool {
        self.hierarchical
            && self
                .scheme
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("platform"))
            && self.authority.is_none()
            && self.device.is_none()
            && self.segments.len() >= 2
            && self.absolute_path
            && (self.segments[0] == "resource" || self.segments[0] == "plugin")
    }

    pub fn last_segment(&self) -> Option<&str> {
        self.segments.last().map(|s| s.as_str())
    }

    /// `fileExtension()`: the text after the last `.` of the last segment.
    pub fn file_extension(&self) -> Option<&str> {
        let last = self.segments.last()?;
        let i = last.rfind('.')?;
        Some(&last[i + 1..])
    }

    pub fn trim_fragment(&self) -> Uri {
        let mut u = self.clone();
        u.fragment = None;
        u
    }

    pub fn append_fragment(&self, fragment: &str) -> Uri {
        let mut u = self.clone();
        u.fragment = Some(fragment.to_string());
        u
    }

    pub fn with_fragment(&self, fragment: Option<&str>) -> Uri {
        let mut u = self.clone();
        u.fragment = fragment.map(|f| f.to_string());
        u
    }

    /// A file system path for a `file:` URI (or a relative path), percent-decoded.
    pub fn to_file_path(&self) -> Option<String> {
        if !self.is_file() {
            return None;
        }
        let mut s = String::new();
        if let Some(d) = &self.device {
            s.push_str(d);
        }
        if self.absolute_path {
            s.push('/');
        }
        s.push_str(&self.segments.join("/"));
        Some(decode(&s))
    }

    fn scheme_eq(&self, other: &Uri) -> bool {
        match (&self.scheme, &other.scheme) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => true,
            _ => false,
        }
    }

    /// `resolve(base)` with `preserveRootParents = false`.
    pub fn resolve(&self, base: &Uri) -> Uri {
        if !base.hierarchical || base.is_relative() || !self.hierarchical || !self.is_relative() {
            return self.clone();
        }
        let mut authority = self.authority.clone();
        let mut device = self.device.clone();
        let mut absolute = self.absolute_path;
        let mut segments = self.segments.clone();
        let mut query = self.query.clone();
        if self.authority.is_none() {
            authority = base.authority.clone();
            if self.device.is_none() {
                device = base.device.clone();
                if self.has_empty_path() && self.query.is_none() {
                    absolute = base.absolute_path;
                    segments = base.segments.clone();
                    query = base.query.clone();
                } else if self.has_relative_path() {
                    absolute = base.has_absolute_path() || !self.has_empty_path();
                    segments = if absolute {
                        self.merge_path(base, false)
                    } else {
                        Vec::new()
                    };
                }
            }
        }
        Uri {
            scheme: base.scheme.clone(),
            hierarchical: true,
            authority,
            device,
            absolute_path: absolute,
            segments,
            query,
            fragment: self.fragment.clone(),
        }
    }

    fn merge_path(&self, base: &Uri, preserve_root_parents: bool) -> Vec<String> {
        let mut stack: Vec<String> = Vec::new();
        let n = base.segments.len();
        for s in base.segments.iter().take(n.saturating_sub(1)) {
            accumulate(&mut stack, s, preserve_root_parents);
        }
        for s in &self.segments {
            accumulate(&mut stack, s, preserve_root_parents);
        }
        if !stack.is_empty() {
            match self.segments.last() {
                None => stack.push(String::new()),
                Some(last) if last.is_empty() || last == ".." || last == "." => {
                    stack.push(String::new())
                }
                _ => {}
            }
        }
        stack
    }

    /// `deresolve(base, preserveRootParents = true, anyRelPath = true, shorterRelPath = false)`,
    /// the form `XMLHelperImpl.deresolve` uses.
    pub fn deresolve(&self, base: &Uri) -> Uri {
        self.deresolve_with(base, true, true, false)
    }

    pub fn deresolve_with(
        &self,
        base: &Uri,
        preserve_root_parents: bool,
        any_rel_path: bool,
        shorter_rel_path: bool,
    ) -> Uri {
        if !base.hierarchical || base.is_relative() || self.is_relative() || !self.hierarchical {
            return self.clone();
        }
        if !self.scheme_eq(base) {
            return self.clone();
        }
        let mut authority = self.authority.clone();
        let mut device = self.device.clone();
        let mut absolute = self.absolute_path;
        let mut segments = self.segments.clone();
        let mut query = self.query.clone();
        if self.authority == base.authority
            && (self.has_device_or_path() || !base.has_device_or_path())
        {
            authority = None;
            if self.device == base.device {
                let has_path = self.has_path();
                let base_has_path = base.has_path();
                if has_path || !base_has_path {
                    device = None;
                    if any_rel_path || shorter_rel_path {
                        if has_path == base_has_path
                            && self.segments == base.segments
                            && self.query == base.query
                        {
                            absolute = false;
                            segments = Vec::new();
                            query = None;
                        } else if has_path && !base_has_path {
                            absolute = false;
                            segments = Vec::new();
                        } else if !self.has_collapsable_segments(preserve_root_parents) {
                            let rel = self.find_relative_path(base, preserve_root_parents);
                            if any_rel_path || self.segments.len() > rel.len() {
                                absolute = false;
                                segments = rel;
                            }
                        }
                    }
                }
            }
        }
        Uri {
            scheme: None,
            hierarchical: true,
            authority,
            device,
            absolute_path: absolute,
            segments,
            query,
            fragment: self.fragment.clone(),
        }
    }

    fn has_collapsable_segments(&self, preserve_root_parents: bool) -> bool {
        let n = self.segments.len();
        self.segments.iter().enumerate().any(|(i, s)| {
            (i < n - 1 && s.is_empty())
                || s == "."
                || (s == ".."
                    && (!preserve_root_parents || (i != 0 && self.segments[i - 1] != "..")))
        })
    }

    fn collapse_segments(&self, preserve_root_parents: bool) -> Vec<String> {
        if !self.has_collapsable_segments(preserve_root_parents) {
            return self.segments.clone();
        }
        let mut stack = Vec::new();
        for s in &self.segments {
            accumulate(&mut stack, s, preserve_root_parents);
        }
        if !stack.is_empty() {
            let last = self.segments.last().unwrap();
            if last.is_empty() || last == ".." || last == "." {
                stack.push(String::new());
            }
        }
        stack
    }

    fn find_relative_path(&self, base: &Uri, preserve_root_parents: bool) -> Vec<String> {
        let start = base.collapse_segments(preserve_root_parents);
        let end = &self.segments;
        let start_count = start.len().saturating_sub(1);
        let end_count = end.len();
        let count = if start_count < end_count {
            start_count
        } else {
            end_count.saturating_sub(1)
        };
        let mut diff = 0;
        while diff < count && start[diff] == end[diff] {
            diff += 1;
        }
        let up = start_count - diff;
        let mut down = end_count - diff;
        if down == 1 && end[end_count - 1].is_empty() {
            down = 0;
        }
        if up + down == 0 {
            return if self.query.is_none() {
                vec![".".to_string()]
            } else {
                Vec::new()
            };
        }
        let mut out = vec!["..".to_string(); up];
        out.extend(end[diff..diff + down].iter().cloned());
        out
    }
}

fn accumulate(stack: &mut Vec<String>, segment: &str, preserve_root_parents: bool) {
    if segment == ".." {
        if stack.is_empty() {
            if preserve_root_parents {
                stack.push(segment.to_string());
            }
        } else if stack.last().unwrap() == ".." {
            stack.push(segment.to_string());
        } else {
            stack.pop();
        }
    } else if !segment.is_empty() && segment != "." {
        stack.push(segment.to_string());
    }
}

fn parse_without_fragment(s: &str) -> Uri {
    let mut u = Uri {
        scheme: None,
        hierarchical: true,
        authority: None,
        device: None,
        absolute_path: false,
        segments: Vec::new(),
        query: None,
        fragment: None,
    };
    let b = s.as_bytes();
    let len = b.len();
    // scheme: up to the first ':' before any '/' or '?'
    let major = b.iter().position(|&c| c == b'/' || c == b':' || c == b'?');
    let mut i = 0;
    let mut is_archive = false;
    let mut is_platform = false;
    if let Some(j) = major
        && b[j] == b':'
    {
        let scheme = &s[..j];
        is_archive = ARCHIVE_SCHEMES
            .iter()
            .any(|a| a.eq_ignore_ascii_case(scheme));
        is_platform = scheme.eq_ignore_ascii_case("platform");
        u.scheme = Some(scheme.to_string());
        i = j + 1;
    }
    let seg_end = |from: usize| -> (usize, u8) {
        let mut k = from;
        while k < len {
            if b[k] == b'/' || b[k] == b'?' {
                return (k, b[k]);
            }
            k += 1;
        }
        (len, b'?')
    };
    let (mut j, mut term) = seg_end(i);
    if is_archive {
        let Some(sep) = s.rfind('!') else {
            // not a valid archive URI: keep it opaque
            u.hierarchical = false;
            u.authority = Some(s[i..].to_string());
            return u;
        };
        u.absolute_path = true;
        u.authority = Some(s[i..sep + 1].to_string());
        i = sep + 2;
        if i > len {
            return u;
        }
        (j, term) = seg_end(i);
    } else if i == j && term == b'/' {
        i += 1;
        (j, term) = seg_end(i);
        if j == i && term == b'/' && i < len {
            i += 1;
            (j, term) = seg_end(i);
            u.authority = Some(s[i..j].to_string());
            i = j;
            if term == b'/' && i < len {
                u.absolute_path = true;
                i += 1;
                (j, term) = seg_end(i);
            } else {
                // authority only, maybe with a query
                if i < len {
                    u.query = Some(s[i + 1..].to_string());
                }
                return u;
            }
        } else {
            u.absolute_path = true;
        }
    } else if u.scheme.is_some() {
        // opaque
        u.hierarchical = false;
        u.authority = Some(s[i..].to_string());
        return u;
    }
    let start = i;
    let seg_len = j - i;
    let mut segments_remain = false;
    i = j;
    if seg_len == 0 {
        if term != b'?' {
            u.segments.push(String::new());
            i += 1;
            (j, term) = seg_end(i);
            segments_remain = true;
        }
    } else if !is_archive && !is_platform && b[j - 1] == b':' {
        u.device = Some(s[start..j].to_string());
        if term == b'?' {
            u.absolute_path = false;
        } else {
            i += 1;
            let (jj, tt) = seg_end(i);
            j = jj;
            term = tt;
            segments_remain = i != j || term == b'/';
        }
    } else {
        u.segments.push(s[start..j].to_string());
        if term != b'?' {
            i += 1;
            (j, term) = seg_end(i);
            segments_remain = true;
        }
    }
    if segments_remain {
        loop {
            u.segments.push(s[i.min(len)..j].to_string());
            i = j;
            if term != b'/' {
                break;
            }
            i += 1;
            (j, term) = seg_end(i);
        }
    }
    if i < len {
        u.query = Some(s[i + 1..].to_string());
    }
    u
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = &self.scheme {
            write!(f, "{s}:")?;
        }
        if !self.hierarchical {
            if let Some(a) = &self.authority {
                f.write_str(a)?;
            }
        } else {
            if let Some(a) = &self.authority {
                if !self.is_archive() {
                    f.write_str("//")?;
                }
                f.write_str(a)?;
            }
            if let Some(d) = &self.device {
                write!(f, "/{d}")?;
            }
            if self.absolute_path {
                f.write_str("/")?;
            }
            f.write_str(&self.segments.join("/"))?;
            if let Some(q) = &self.query {
                write!(f, "?{q}")?;
            }
        }
        if let Some(fr) = &self.fragment {
            write!(f, "#{fr}")?;
        }
        Ok(())
    }
}

/// Percent-encodes a file system path the way `URI.createFileURI` does (segment characters are
/// kept, everything else becomes `%XX` of its UTF-8 bytes).
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        let keep = ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '-' | '_'
                    | '.'
                    | '!'
                    | '~'
                    | '*'
                    | '\''
                    | '('
                    | ')'
                    | ':'
                    | '@'
                    | '&'
                    | '='
                    | '+'
                    | '$'
                    | ','
                    | ';'
                    | '/'
            );
        if keep {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

/// `URI.decode`: `%XX` escapes become bytes (UTF-8); malformed escapes stay.
pub fn decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_strings() {
        for s in [
            "file:/home/x/a.ecore",
            "http://www.eclipse.org/emf/2002/Ecore",
            "http:///simpleuml.ecore",
            "platform:/plugin/org.eclipse.ocl/model/OCL.ecore#//utilities/Visitable",
            "pathmap://PCM_MODELS/Palladio.resourcetype",
            "a.ecore#//X",
            "../b/c.xmi#/0",
            "#//X",
            "urn:abc",
            "extent:/1",
        ] {
            assert_eq!(Uri::parse(s).to_string(), s);
        }
    }

    #[test]
    fn deresolve_and_resolve() {
        let base = Uri::parse("file:/a/b/c.xmi");
        let t = Uri::parse("file:/a/b/d.xmi#//X");
        assert_eq!(t.deresolve(&base).to_string(), "d.xmi#//X");
        let t = Uri::parse("file:/a/e/d.xmi#/0");
        assert_eq!(t.deresolve(&base).to_string(), "../e/d.xmi#/0");
        let t = Uri::parse("file:/a/b/c.xmi#/0");
        assert_eq!(t.deresolve(&base).to_string(), "#/0");
        let t = Uri::parse("http://www.eclipse.org/emf/2002/Ecore#//EString");
        assert_eq!(
            t.deresolve(&base).to_string(),
            "http://www.eclipse.org/emf/2002/Ecore#//EString"
        );
        let r = Uri::parse("../e/d.xmi#/0").resolve(&base);
        assert_eq!(r.to_string(), "file:/a/e/d.xmi#/0");
        let r = Uri::parse("d.xmi").resolve(&base);
        assert_eq!(r.to_string(), "file:/a/b/d.xmi");
        assert_eq!(Uri::file("/x y/a.ecore").to_string(), "file:/x%20y/a.ecore");
        assert_eq!(
            Uri::file("/x y/a.ecore").to_file_path().unwrap(),
            "/x y/a.ecore"
        );
    }
}
