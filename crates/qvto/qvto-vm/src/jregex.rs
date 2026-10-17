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

/// Java's `Matcher.find` loop: the matches in order, where (unlike `regex`'s iterators) an
/// empty match right after a non-empty one is also reported; after an empty match the search
/// resumes one character further.
pub fn find_all<'t>(re: &Regex, text: &'t str) -> Vec<regex::Captures<'t>> {
    let mut out = Vec::new();
    let mut from = 0;
    while from <= text.len() {
        let Some(c) = re.captures_at(text, from) else {
            break;
        };
        let m = c.get(0).expect("group 0");
        from = if m.end() == m.start() {
            match text[m.end()..].chars().next() {
                Some(ch) => m.end() + ch.len_utf8(),
                None => text.len() + 1,
            }
        } else {
            m.end()
        };
        out.push(c);
    }
    out
}

/// `Matcher.appendReplacement`'s expansion of a Java replacement string for one match: `\x` is
/// `x`; `$n` takes the first digit and further digits while the number is a group of the
/// pattern (a first digit beyond the groups: `IndexOutOfBoundsException: No group n`);
/// `${name}` a named group; an unmatched group appends nothing.
fn expand(rep: &str, c: &regex::Captures<'_>, re: &Regex, out: &mut String) -> Result<(), String> {
    let groups = re.captures_len() - 1;
    let chars: Vec<char> = rep.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                i += 1;
                match chars.get(i) {
                    Some(&x) => out.push(x),
                    None => {
                        return Err(
                            "IllegalArgumentException: character to be escaped is missing".into(),
                        );
                    }
                }
                i += 1;
            }
            '$' => {
                i += 1;
                let Some(&d) = chars.get(i) else {
                    return Err(
                        "IllegalArgumentException: Illegal group reference: group index is missing"
                            .into(),
                    );
                };
                if d == '{' {
                    let end = chars[i..].iter().position(|&x| x == '}').map(|k| i + k);
                    let Some(end) = end else {
                        return Err("IllegalArgumentException: named capturing group is missing trailing '}'".into());
                    };
                    let name: String = chars[i + 1..end].iter().collect();
                    if re.capture_names().flatten().all(|n| n != name) {
                        return Err(format!(
                            "IllegalArgumentException: No group with name {{{name}}}"
                        ));
                    }
                    if let Some(m) = c.name(&name) {
                        out.push_str(m.as_str());
                    }
                    i = end + 1;
                    continue;
                }
                let Some(mut n) = d.to_digit(10).map(|x| x as usize) else {
                    return Err("IllegalArgumentException: Illegal group reference".into());
                };
                if n > groups {
                    return Err(format!("IndexOutOfBoundsException: No group {n}"));
                }
                i += 1;
                while let Some(x) = chars.get(i).and_then(|c| c.to_digit(10)) {
                    let m = n * 10 + x as usize;
                    if m > groups {
                        break;
                    }
                    n = m;
                    i += 1;
                }
                if let Some(m) = c.get(n) {
                    out.push_str(m.as_str());
                }
            }
            x => {
                out.push(x);
                i += 1;
            }
        }
    }
    Ok(())
}

/// `Matcher.replaceAll`/`replaceFirst` with a Java replacement string ([`find_all`],
/// [`expand`]); the replacement is only looked at when there is a match.
pub fn replace(re: &Regex, text: &str, rep: &str, all: bool) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for c in find_all(re, text) {
        let m = c.get(0).expect("group 0");
        out.push_str(&text[last..m.start()]);
        expand(rep, &c, re, &mut out)?;
        last = m.end();
        if !all {
            break;
        }
    }
    out.push_str(&text[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_replacement() {
        let re = compile_full("[a-z]+").unwrap();
        assert!(re.is_match("abc") && !re.is_match("abc1"));
        let f =
            |p: &str, t: &str, r: &str, all: bool| replace(&compile_find(p).unwrap(), t, r, all);
        // java.lang.String.replaceAll / replaceFirst (JDK 21)
        assert_eq!(f(".*", "ab", "-", true).unwrap(), "--");
        assert_eq!(f(".*", "", "-", true).unwrap(), "-");
        assert_eq!(f("x*", "ab", "-", true).unwrap(), "-a-b-");
        assert_eq!(f("a|", "aab", "<$0>", true).unwrap(), "<a><a><>b<>");
        assert_eq!(f(".*", "ab", "-", false).unwrap(), "-");
        assert_eq!(f("(a)(b)", "xaby", "$2$1", true).unwrap(), "xbay");
        assert_eq!(f("(a)", "a", "$12", true).unwrap(), "a2");
        assert_eq!(f("(a)", "a", "\\$x", true).unwrap(), "$x");
        assert!(f("a", "a", "$1", true).is_err());
        assert_eq!(f("b", "a", "$1", true).unwrap(), "a");
        assert!(f("a", "a", "$", true).is_err());
        assert!(f("a", "a", "\\", true).is_err());
    }
}
