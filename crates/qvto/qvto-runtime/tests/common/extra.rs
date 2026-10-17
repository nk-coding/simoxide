//! Requests other than hashes and collection scripts: number formatting and parsing, OCL
//! arithmetic, Java string operations and character data.

use super::{TapeEnv, decode, encode, hex16, unhex16};
use qvto_runtime::eq::ocl_equal;
use qvto_runtime::identity::Mode;
use qvto_runtime::java::{chars, fmt, parse, string as js};
use qvto_runtime::num::{self, NumOp};
use qvto_runtime::{Str, Value};

fn s16(h: &str) -> Str {
    Str::from_utf16(&unhex16(h))
}

pub fn eval(cmd: &str, arg: &str) -> String {
    match cmd {
        "dtoa" => fmt::double_to_string(f64::from_bits(u64::from_str_radix(arg, 16).unwrap())),
        "ftoa" => fmt::float_to_string(f32::from_bits(u32::from_str_radix(arg, 16).unwrap())),
        "dparse" => match parse::parse_double(&unhex16(arg)) {
            Some(d) => format!("{:x}", d.to_bits()),
            None => "NFE".into(),
        },
        "iparse" => parse::parse_int(&unhex16(arg)).map_or("NFE".into(), |v| v.to_string()),
        "lparse" => parse::parse_long(&unhex16(arg)).map_or("NFE".into(), |v| v.to_string()),
        "num" => num_req(arg),
        "str" => str_req(arg),
        "char" => {
            let u: u16 = arg.parse().unwrap();
            format!(
                "{}{}{} {} {} {}",
                if chars::is_whitespace(u) { "W" } else { "-" },
                if chars::is_letter(u) { "L" } else { "-" },
                if chars::is_letter_or_digit(u) {
                    "D"
                } else {
                    "-"
                },
                chars::digit10(u).map_or(-1, |d| d as i64),
                chars::to_upper_unit(u),
                chars::to_lower_unit(u)
            )
        }
        "cp" => {
            let c: u32 = arg.parse().unwrap();
            let s = Str::from_utf16(&char_units(c));
            format!(
                "{} {} {} {} {}",
                hex16(js::to_upper_case(&s).units()),
                hex16(js::to_lower_case(&s).units()),
                chars::to_upper_cp(c),
                chars::to_lower_cp(c),
                if chars::is_cased(c) { "C" } else { "-" }
            )
        }
        _ => format!("UNIMPLEMENTED {cmd}"),
    }
}

fn char_units(c: u32) -> Vec<u16> {
    if c >= 0x10000 {
        let x = c - 0x10000;
        vec![0xd800 | (x >> 10) as u16, 0xdc00 | (x & 0x3ff) as u16]
    } else {
        vec![c as u16]
    }
}

fn num_req(arg: &str) -> String {
    let t: Vec<&str> = arg.split(' ').collect();
    let env = TapeEnv::new(Mode::Compat);
    let a = decode(t[1], &env, &[]);
    let b = t.get(2).map(|x| decode(x, &env, &[]));
    let op = match t[0] {
        "+" => Some(NumOp::Plus),
        "-" => Some(NumOp::Minus),
        "*" => Some(NumOp::Times),
        "/" => Some(NumOp::Divide),
        "div" => Some(NumOp::Div),
        "mod" => Some(NumOp::Mod),
        "max" => Some(NumOp::Max),
        "min" => Some(NumOp::Min),
        "<" => Some(NumOp::Lt),
        ">" => Some(NumOp::Gt),
        "<=" => Some(NumOp::Le),
        ">=" => Some(NumOp::Ge),
        _ => None,
    };
    let r = match (op, t[0]) {
        (Some(op), _) => num::binary(op, &a, b.as_ref().unwrap(), false, false).unwrap(),
        (None, "neg") => num::negate(&a, false),
        (None, "abs") => num::abs(&a, false),
        (None, "floor") => num::floor(&a, false),
        (None, "round") => num::round(&a, false),
        (None, "toString") => Value::str(&qvto_runtime::display::to_java_string(&a, &env)),
        (None, "=") => Value::Bool(ocl_equal(&a, b.as_ref().unwrap(), &env)),
        (None, "<>") => Value::Bool(!ocl_equal(&a, b.as_ref().unwrap(), &env)),
        _ => panic!("num op {}", t[0]),
    };
    encode(&r)
}

fn str_req(arg: &str) -> String {
    let a: Vec<&str> = arg.split(' ').collect();
    let s = s16(a[1]);
    let arg = |i: usize| s16(a[i]);
    match a[0] {
        "hash" => s.java_hash().to_string(),
        "len" => s.len16().to_string(),
        "upper" => hex16(js::to_upper_case(&s).units()),
        "lower" => hex16(js::to_lower_case(&s).units()),
        "trim" => hex16(js::trim(&s).units()),
        "cmp" => (qvto_runtime::eq::compare_str(&s, &arg(2)) as i32).to_string(),
        "eqic" => if js::equals_ignore_case(&s, &arg(2)) {
            "T"
        } else {
            "F"
        }
        .into(),
        "idx" => js::index_of(&s, &arg(2)).to_string(),
        "lidx" => js::last_index_of(&s, &arg(2)).to_string(),
        "repl" => hex16(js::replace(&s, &arg(2), &arg(3)).units()),
        "sw" => if js::starts_with(&s, &arg(2)) {
            "T"
        } else {
            "F"
        }
        .into(),
        "ew" => if js::ends_with(&s, &arg(2)) { "T" } else { "F" }.into(),
        "sub" => match (a[2].parse::<usize>().ok(), a[3].parse::<usize>().ok())
            .0
            .zip(a[3].parse::<usize>().ok())
            .and_then(|(b, e)| js::substring(&s, b, e))
        {
            Some(x) => hex16(x.units()),
            None => "EXC StringIndexOutOfBoundsException".into(),
        },
        "tok" => {
            let d = if a.len() > 2 {
                arg(2)
            } else {
                Str::new(js::DEFAULT_DELIMS)
            };
            let toks: Vec<String> = js::tokenize(&s, &d, false)
                .iter()
                .map(|t| hex16(t.units()))
                .collect();
            format!("[{}]", toks.join(","))
        }
        other => format!("UNIMPLEMENTED str {other}"),
    }
}
