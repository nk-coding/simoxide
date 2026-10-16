//! Requests other than hashes and collection scripts.

pub fn eval(cmd: &str, _arg: &str) -> String {
    format!("UNIMPLEMENTED {cmd}")
}
