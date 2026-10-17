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
    interm: &'a crate::objects::IntermInfo,
    /// Objects of Ecore's own classes: their generated class and `toString` attributes.
    ecore: &'a dyn Fn(ObjRef) -> Option<(String, String)>,
}

impl ObjectFormat for Fmt<'_> {
    fn object(&self, o: ObjRef, env: &dyn HashEnv, out: &mut String) {
        if let Some(c) = (self.class_of)(o)
            && self.interm.is_interm_class(c)
        {
            // `IntermediateClassInstance.toString`
            out.push_str(&self.prog.ir.meta.class(c).name);
            return;
        }
        let key = IdentityKey::Obj(o);
        if let Some((class, attrs)) = (self.ecore)(o) {
            // the generated `toString` of Ecore's classes (`EClassImpl@hash (name: X) ...`);
            // an `EEnumLiteral` is an `Enumerator` and prints its literal
            if class.is_empty() {
                out.push_str(&attrs);
            } else {
                let _ = write!(out, "{class}@{:x}{attrs}", env.identity_hash(key));
            }
            return;
        }
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
            None => out.push('?'),
        }
    }
    fn type_name(&self, t: TypeRef, out: &mut String) {
        use qvto_types::Type;
        // classic OCL's primitive type objects (`PrimitiveTypeImpl.toString`)
        let prim = match self.prog.ir.types.get(qvto_types::TypeId(t.0)) {
            Type::Integer => Some(("Integer", "java.lang.Integer")),
            Type::Real => Some(("Real", "java.lang.Double")),
            Type::String => Some(("String", "java.lang.String")),
            Type::Boolean => Some(("Boolean", "java.lang.Boolean")),
            Type::UnlimitedNatural => Some(("UnlimitedNatural", "java.lang.Integer")),
            _ => None,
        };
        match prim {
            Some((n, c)) => {
                let _ = write!(
                    out,
                    "org.eclipse.ocl.ecore.impl.PrimitiveTypeImpl@1 (name: {n}) (instanceClassName: {c}) (serializable: true)"
                );
            }
            None => out.push_str(&crate::types::type_simple_name(
                self.prog,
                qvto_types::TypeId(t.0),
            )),
        }
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
        if let Value::Obj(o) = v
            && o.is_local()
        {
            let lo = &self.local[o.index() as usize];
            if let Some((ok, exc)) = &lo.status {
                out.push_str(if *ok { "SUCCESS" } else { "FAILED" });
                if !exc.is_null() {
                    out.push_str(" raised: (");
                    self.write_value(exc, out);
                    out.push(')');
                }
                return;
            }
            if lo.exception.is_some() {
                out.push_str(&self.exception_text(*o));
                return;
            }
        }
        let class_of = |o: ObjRef| self.class_of(&Value::Obj(o));
        let ecore = |o: ObjRef| self.ecore_to_string(o);
        let f = Fmt {
            prog: self.prog,
            class_of: &class_of,
            interm: &self.interm_info,
            ecore: &ecore,
        };
        java_to_string(v, &self.env, &f, out);
    }

    /// The generated `toString` of an object of one of Ecore's classes: its Java class and the
    /// attribute groups each implementation class in its hierarchy appends.
    fn ecore_to_string(&self, o: ObjRef) -> Option<(String, String)> {
        if o.is_local() {
            return None;
        }
        let meta = &self.prog.ir.meta;
        let c = self.class_of(&Value::Obj(o))?;
        let ecore = meta.package_by_ns_uri("http://www.eclipse.org/emf/2002/Ecore")?;
        let ci = meta.class(c);
        if ci.package != ecore {
            return None;
        }
        let h = S::Obj::from_u32(o.index());
        let is = |name: &str| {
            meta.class_by_name(ecore, name)
                .is_some_and(|x| x == c || meta.is_subclass(c, x))
        };
        let val = |name: &str| -> String {
            let Some(f) = meta.feature_by_name(c, name) else {
                return "null".into();
            };
            match self.store.get(h, f) {
                qvto_model::Datum::Null => "null".into(),
                qvto_model::Datum::Bool(b) => b.to_string(),
                qvto_model::Datum::Int(i) => i.to_string(),
                qvto_model::Datum::Long(i) => i.to_string(),
                qvto_model::Datum::Str(s) => s.to_string(),
                _ => "null".into(),
            }
        };
        let group = |names: &[&str]| -> String {
            let parts: Vec<String> = names.iter().map(|n| format!("{n}: {}", val(n))).collect();
            format!(" ({})", parts.join(", "))
        };
        if is("EEnumLiteral") {
            return Some((String::new(), val("literal")));
        }
        let mut s = String::new();
        if is("ENamedElement") {
            s += &group(&["name"]);
        }
        if is("EClassifier") {
            s += &group(&["instanceClassName"]);
        }
        if is("EClass") {
            s += &group(&["abstract", "interface"]);
        }
        if is("EDataType") {
            s += &group(&["serializable"]);
        }
        if is("ETypedElement") {
            s += &group(&["ordered", "unique", "lowerBound", "upperBound"]);
        }
        if is("EStructuralFeature") {
            s += &group(&[
                "changeable",
                "volatile",
                "transient",
                "defaultValueLiteral",
                "unsettable",
                "derived",
            ]);
        }
        if is("EAttribute") {
            s += &group(&["iD"]);
        }
        if is("EReference") {
            s += &group(&["containment", "resolveProxies"]);
        }
        if is("EPackage") {
            s += &group(&["nsURI", "nsPrefix"]);
        }
        if is("EAnnotation") {
            s += &group(&["source"]);
        }
        if is("EStringToStringMapEntry") {
            s += &group(&["key", "value"]);
        }
        Some((format!("org.eclipse.emf.ecore.impl.{}Impl", ci.name), s))
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
        // ExceptionClassInstance.toString
        let mut s = name.to_string();
        if let Some((m, stack)) = &lo.exception {
            if let Some(m) = m {
                s.push_str(" : ");
                s.push_str(m);
                s.push('\n');
            }
            for (i, e) in stack.iter().enumerate() {
                if i > 0 {
                    s.push('\n');
                }
                s.push_str("\tat ");
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
        if i > 0 && (s.len() - i).is_multiple_of(3) {
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
