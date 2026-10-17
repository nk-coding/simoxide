//! Java `Double.toString` (JDK >= 19, Raffaello Giulietti's shortest-decimal specification), as used by
//! every number in `palladio-trace/1` files (`docs/guide/formats.md` §1): byte-buffer forms of
//! `simoxide_jcompat::fmt`, which also has [`exact`], the literal big-integer implementation of the
//! javadoc that serves as the subnormal fallback and the property-test oracle.

pub use simoxide_jcompat::fmt::{Decimal, MAX_LEN, exact, shortest};

/// `Double.toString(x)` of Java >= 19 (NaN -> `NaN`, infinities -> `Infinity` / `-Infinity`).
pub fn to_string(x: f64) -> String {
    simoxide_jcompat::fmt::double_to_string(x)
}

/// Appends `Double.toString(x)` to `out`.
#[inline]
pub fn write(out: &mut Vec<u8>, x: f64) {
    simoxide_jcompat::fmt::write_double(out, x);
}

/// Appends the trace/CSV form: like [`write`], but NaN and infinities as JSON strings (`"NaN"`).
pub fn write_json(out: &mut Vec<u8>, x: f64) {
    if x.is_finite() {
        write(out, x);
    } else {
        out.push(b'"');
        write(out, x);
        out.push(b'"');
    }
}

/// Java's layout of a decimal: plain for `1e-3 <= d < 1e7`, else computerized scientific notation.
#[inline]
pub fn format_decimal(out: &mut Vec<u8>, d: Decimal) {
    simoxide_jcompat::fmt::write_decimal(out, d);
}

pub(crate) fn itoa_buf() -> [u8; 24] {
    [0u8; 24]
}

/// Formats an integer into `buf`, returns the used slice.
pub(crate) fn fmt_i64(buf: &mut [u8; 24], v: i64) -> &[u8] {
    let neg = v < 0;
    let mut u = v.unsigned_abs();
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    if neg {
        i -= 1;
        buf[i] = b'-';
    }
    &buf[i..]
}
