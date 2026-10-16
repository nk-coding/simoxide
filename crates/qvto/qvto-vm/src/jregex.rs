//! Java regular expressions (`java.util.regex`) on Rust's `regex` crate: the patterns both
//! accept with the same meaning, `Pattern.matches` as an anchored match, and Java's replacement
//! syntax (`$n`, `\x`). Patterns with features `regex` lacks (look-around, back references,
//! possessive quantifiers) fail to compile; the operation yields invalid (DESIGN §6.2 row 14:
//! a Java-compatible engine is later work).

use regex::Regex;

/// A pattern for `String.matches`/`Pattern.matches` (the whole input must match).
pub fn compile_full(p: &str) -> Result<Regex, String> {
    Regex::new(&format!("^(?:{})$", translate(p)))
        .map_err(|e| format!("PatternSyntaxException: {e}"))
}

/// A pattern for `replaceAll`/`replaceFirst` (unanchored).
pub fn compile_find(p: &str) -> Result<Regex, String> {
    Regex::new(&translate(p)).map_err(|e| format!("PatternSyntaxException: {e}"))
}

/// Java syntax that means something else in `regex`: none of the common constructs; kept as a
/// hook for translations.
fn translate(p: &str) -> String {
    p.to_string()
}

/// Java's replacement string (`Matcher.appendReplacement`): `$n` group references, `\x` the
/// character `x`, as a `regex` replacement (`${n}`, `$$`).
pub fn replacement(r: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut it = r.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('$') => out.push_str("$$"),
                Some(x) => out.push(x),
                None => {
                    return Err(
                        "IllegalArgumentException: character to be escaped is missing".into(),
                    );
                }
            },
            '$' => {
                let mut n = String::new();
                while let Some(d) = it.peek().copied().filter(|d| d.is_ascii_digit()) {
                    n.push(d);
                    it.next();
                }
                if n.is_empty() {
                    return Err("IllegalArgumentException: Illegal group reference".into());
                }
                out.push_str(&format!("${{{n}}}"));
            }
            x => out.push(x),
        }
    }
    Ok(out)
}

/// `Matcher.replaceAll`/`replaceFirst`: Java's `find` loop, which (unlike `regex`'s
/// iterators) also reports an empty match right after a non-empty one (`"ab".replaceAll(".*",
/// "-")` is `"--"`); after an empty match the search resumes one character further. `rep` is a
/// [`replacement`].
pub fn replace(re: &Regex, text: &str, rep: &str, all: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut from = 0;
    while from <= text.len() {
        let Some(c) = re.captures_at(text, from) else {
            break;
        };
        let m = c.get(0).expect("group 0");
        out.push_str(&text[last..m.start()]);
        c.expand(rep, &mut out);
        last = m.end();
        if !all {
            break;
        }
        from = if m.end() == m.start() {
            match text[m.end()..].chars().next() {
                Some(ch) => m.end() + ch.len_utf8(),
                None => break,
            }
        } else {
            m.end()
        };
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_replacement() {
        assert_eq!(replacement("a$1b").unwrap(), "a${1}b");
        assert_eq!(replacement("\\$x").unwrap(), "$$x");
        assert!(replacement("$").is_err());
        let re = compile_full("[a-z]+").unwrap();
        assert!(re.is_match("abc") && !re.is_match("abc1"));
        let f = |p: &str, t: &str, r: &str, all: bool| {
            replace(&compile_find(p).unwrap(), t, &replacement(r).unwrap(), all)
        };
        // java.lang.String.replaceAll / replaceFirst (JDK 21)
        assert_eq!(f(".*", "ab", "-", true), "--");
        assert_eq!(f(".*", "", "-", true), "-");
        assert_eq!(f("x*", "ab", "-", true), "-a-b-");
        assert_eq!(f("a|", "aab", "<$0>", true), "<a><a><>b<>");
        assert_eq!(f(".*", "ab", "-", false), "-");
        assert_eq!(f("(a)(b)", "xaby", "$2$1", true), "xbay");
    }
}
