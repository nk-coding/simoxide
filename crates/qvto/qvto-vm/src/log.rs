//! `log`, `assert` and the string forms of values (`String.valueOf`, EMF's `toString`).

use crate::error::{ErrorKind, VmResult};
use crate::vm::{Diag, ObjHandle, Vm};
use qvto_bytecode::Site;
use qvto_model::ModelStore;
use qvto_runtime::display::{ObjectFormat, java_to_string};
use qvto_runtime::{EnumLit, HashEnv, IdentityKey, ObjRef, TypeRef, Value};
use qvto_sema::ir::AssertSeverity;
use std::fmt::Write;

/// Formats objects, enum literals and types with the program's metamodels.
struct Fmt<'a> {
    prog: &'a qvto_bytecode::Program,
    class_of: &'a dyn Fn(ObjRef) -> Option<qvto_model::ClassId>,
}

impl ObjectFormat for Fmt<'_> {
    fn object(&self, o: ObjRef, env: &dyn HashEnv, out: &mut String) {
        let key = IdentityKey::Obj(o);
        let _ = write!(out, "{}@{:x}", env.class_name(key), env.identity_hash(key));
        if let Some(c) = (self.class_of)(o) {
            let meta = &self.prog.ir.meta;
            let ci = meta.class(c);
            let ck = IdentityKey::Type(TypeRef(0x4000_0000 | c.0));

            let _ = write!(
                out,
                " (eClass: org.eclipse.emf.ecore.impl.EClassImpl@{:x} (name: {}) (instanceClassName: {}) (abstract: {}, interface: {}))",
                env.identity_hash(ck),
                ci.name,
                ci.instance_class_name.as_deref().unwrap_or("null"),
                ci.is_abstract,
                ci.is_interface
            );
        }
    }
    fn enum_literal(&self, e: EnumLit, out: &mut String) {
        let meta = &self.prog.ir.meta;
        let en = meta.enumeration(qvto_model::EnumId(e.enum_id));
        match en.literals.get(e.literal as usize) {
            Some(l) => out.push_str(&l.literal),
            None => out.push_str("?"),
        }
    }
    fn type_name(&self, t: TypeRef, out: &mut String) {
        out.push_str(&crate::types::type_simple_name(
            self.prog,
            qvto_types::TypeId(t.0),
        ));
    }
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// `String.valueOf(v)` (`toString()` with `*` for UnlimitedNatural).
    pub(crate) fn to_java_string(&self, v: &Value, unlimited: bool) -> String {
        if unlimited && matches!(v, Value::Int(-1)) {
            return "*".into();
        }
        let mut out = String::new();
        self.write_value(v, &mut out);
        out
    }

    fn write_value(&self, v: &Value, out: &mut String) {
        let class_of = |o: ObjRef| self.class_of(&Value::Obj(o));
        let f = Fmt {
            prog: self.prog,
            class_of: &class_of,
        };
        java_to_string(v, &self.env, &f, out);
    }

    /// One record of the transformation's log (`WriterLog`, records end with a newline).
    pub(crate) fn log_line(&mut self, s: &str) {
        self.log.push_str(s);
        self.log.push('\n');
    }

    /// The message text of a log record: `<null>`, `<Invalid>`, else `String.valueOf`.
    fn log_message(&self, v: &Value) -> String {
        match v {
            Value::Null => "<null>".into(),
            Value::Invalid => "<Invalid>".into(),
            v => self.to_java_string(v, false),
        }
    }

    fn log_element(&self, v: &Value) -> Option<String> {
        match v {
            Value::Null => None,
            Value::Invalid => Some("<Invalid>".into()),
            Value::Obj(o) if o.is_local() && self.local[o.index() as usize].exception.is_some() => {
                Some(self.exception_text(*o))
            }
            v => Some(self.to_java_string(v, false)),
        }
    }

    fn exception_text(&self, o: ObjRef) -> String {
        let lo = &self.local[o.index() as usize];
        let name = &self.prog.ir.meta.class(lo.class).name;
        let mut s = name.to_string();
        if let Some((m, stack)) = &lo.exception {
            if let Some(m) = m {
                s.push_str(" : ");
                s.push_str(m);
            }
            for e in stack {
                s.push_str("\n\tat ");
                s.push_str(&e.to_string());
            }
        }
        s
    }

    /// `level` as `strictConvertNumber(level, Integer)`.
    fn log_level(&self, v: &Value) -> Option<i32> {
        v.as_jnum().map(|n| n.as_i32())
    }

    /// Writes a log record from the registers `at` (level), `at + 1` (message), `at + 2`
    /// (element); returns the message text. `prefix`: an assertion's location.
    pub(crate) fn do_log(
        &mut self,
        at: usize,
        has_level: bool,
        has_element: bool,
        prefix: Option<&str>,
    ) -> String {
        let level = if has_level {
            self.log_level(&self.regs[at])
        } else {
            None
        };
        let message = self.log_message(&self.regs[at + 1]);
        let entry = match prefix {
            Some(p) => format!("{p} : {message}"),
            None => message.clone(),
        };
        let element = if has_element {
            self.log_element(&self.regs[at + 2])
        } else {
            None
        };
        let line = match (level, element) {
            (None, None) => entry,
            (None, Some(e)) => format!("{entry}, data: {e}"),
            (Some(l), None) => format!("Level {} - {entry}", group(l)),
            (Some(l), Some(e)) => format!("Level {} - {entry}, data: {e}", group(l)),
        };
        self.log_line(&line);
        if prefix.is_none() {
            self.diags.push(Diag {
                severity: 0,
                code: level.unwrap_or(0),
                message: message.clone(),
            });
        }
        message
    }

    pub(crate) fn assert_failed(&mut self, site: &Site, at: usize) -> VmResult<()> {
        let Site::Assert {
            severity,
            location,
            has_log,
            has_level,
            has_element,
            silent,
        } = site
        else {
            unreachable!()
        };
        let sev = match severity {
            AssertSeverity::Warning => "warning",
            AssertSeverity::Error => "error",
            AssertSeverity::Fatal => "fatal",
        };
        let message = format!("ASSERT [{sev}] failed at ({location})");
        let log_message = if *has_log {
            Some(self.do_log(at, *has_level, *has_element, Some(&message)))
        } else {
            if !*silent {
                self.log_line(&message);
            }
            None
        };
        if *severity == AssertSeverity::Fatal {
            return Err(self.error(
                ErrorKind::AssertionFailed,
                Some(log_message.unwrap_or_else(|| "Fatal assertion failed".into())),
            ));
        }
        self.diags.push(Diag {
            severity: if *severity == AssertSeverity::Warning {
                2
            } else {
                4
            },
            code: 150,
            message: log_message.unwrap_or_else(|| "Non-fatal assertion failed".into()),
        });
        Ok(())
    }
}

/// `MessageFormat` of an Integer in the en_US locale (grouping by thousands).
fn group(n: i32) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    #[test]
    fn grouping() {
        assert_eq!(super::group(1234567), "1,234,567");
        assert_eq!(super::group(-1000), "-1,000");
        assert_eq!(super::group(12), "12");
    }
}
