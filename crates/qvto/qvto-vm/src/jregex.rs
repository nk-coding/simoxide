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
    }
}
