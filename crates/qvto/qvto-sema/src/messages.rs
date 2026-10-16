//! Eclipse's message texts ([`validation`], [`compiler`], [`ocl`]), generated from the
//! `.properties` files (`examples/gen_messages.rs`), and [`bind`], a port of Eclipse's
//! `NLS.bind`.

#[path = "messages_data.rs"]
#[rustfmt::skip]
mod data;

pub use data::{compiler, ocl, validation};

/// `org.eclipse.osgi.util.NLS.bind`: `{n}` is replaced by the n-th argument
/// (`<missing argument>` when absent), `''` is a quote, text between single quotes is literal.
pub fn bind(message: &str, args: &[&str]) -> String {
    let chars: Vec<char> = message.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(message.len() + 16);
    let mut i = 0;
    while i < len {
        let c = chars[i];
        match c {
            '{' => {
                let Some(close) = chars[i..].iter().position(|&x| x == '}').map(|p| p + i) else {
                    out.push(c);
                    i += 1;
                    continue;
                };
                if i + 1 >= len {
                    out.push(c);
                    i += 1;
                    continue;
                }
                let num: String = chars[i + 1..close].iter().collect();
                match num.trim().parse::<usize>() {
                    Ok(n) if n < args.len() => out.push_str(args[n]),
                    _ => out.push_str("<missing argument>"),
                }
                i = close + 1;
            }
            '\'' => {
                let next = i + 1;
                if next >= len {
                    out.push(c);
                    i += 1;
                    continue;
                }
                if chars[next] == '\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                match chars[next..].iter().position(|&x| x == '\'') {
                    None => {
                        // NLS.bind: an unmatched quote is dropped, the rest copied
                        out.extend(&chars[next..]);
                        i = len;
                    }
                    Some(p) => {
                        out.extend(&chars[next..next + p]);
                        i = next + p + 1;
                    }
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nls_bind() {
        assert_eq!(
            bind(validation::UnresolvedModuleReference, &["a.b"]),
            "Can't resolve module reference 'a.b'"
        );
        assert_eq!(bind("x {0} {1}", &["a"]), "x a <missing argument>");
        assert_eq!(
            bind(ocl::OperationNotFound_ERROR_, &["foo(Integer)", "String"]),
            "Cannot find operation (foo(Integer)) for the type (String)"
        );
    }
}
