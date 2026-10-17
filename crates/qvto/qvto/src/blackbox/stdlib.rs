//! QVTo's own deprecated blackbox libraries, the "legacy libraries" of
//! `org.eclipse.m2m.qvt.oml.ocl` (`StringLibrary` as unit `Strings`, `EmfToolsLibrary` as unit
//! `emf.tools`): their signatures and Rust ports of their Java methods.
//!
//! Ported from Eclipse QVTo (EPL-2.0): `org.eclipse.m2m.qvt.oml.ocl.legacy.libraries`.

use super::java;
use super::{Catalog, NativeCall, NativeResult, Natives};
use qvto_model::{Classifier, ModelStore};
use qvto_runtime::coll::CollKind;
use qvto_runtime::Value;
use qvto_vm::{ObjHandle, Vm};

/// The signatures of `Strings` and `emf.tools` (environment `stdlib`).
pub const SIGNATURES: &str = include_str!("../../data/stdlib-blackboxes.tsv");

/// The units of the standard blackbox libraries.
pub fn catalog() -> Catalog {
    Catalog::parse(SIGNATURES).expect("stdlib-blackboxes.tsv")
}

const STRINGS: &str = "org.eclipse.m2m.qvt.oml.ocl.legacy.libraries.StringLibrary";
const EMF_TOOLS: &str = "org.eclipse.m2m.qvt.oml.ocl.legacy.libraries.EmfToolsLibrary";

/// Registers the implementations of `Strings` and `emf.tools`.
pub fn register<S: ModelStore + 'static>(n: &mut Natives<S>)
where
    S::Obj: ObjHandle,
{
    // ---- StringLibrary (contextual on String) ----
    n.java(
        &format!("public java.lang.Boolean {STRINGS}.startsWith(java.lang.String,java.lang.String,java.lang.Integer)"),
        |_vm, c| {
            let (Some(s), Some(p), Some(o)) =
                (java::utf16(&c.self_), java::utf16(&c.args[0]), java::int(&c.args[1]))
            else {
                return Ok(Value::Bool(false));
            };
            Ok(Value::Bool(java::starts_with(&s, &p, o)))
        },
    );
    n.java(
        &format!("public java.lang.Integer {STRINGS}.indexOf(java.lang.String,java.lang.String,java.lang.Integer)"),
        |_vm, c| {
            let (Some(s), Some(x), Some(from)) =
                (java::utf16(&c.self_), java::utf16(&c.args[0]), java::int(&c.args[1]))
            else {
                return Ok(Value::Int(-1));
            };
            Ok(Value::Int(java::index_of(&s, &x, from)))
        },
    );
    n.java(
        &format!("public java.lang.Integer {STRINGS}.lastIndexOf(java.lang.String,java.lang.String,java.lang.Integer)"),
        |_vm, c| {
            let (Some(s), Some(x), Some(from)) =
                (java::utf16(&c.self_), java::utf16(&c.args[0]), java::int(&c.args[1]))
            else {
                return Ok(Value::Int(-1));
            };
            Ok(Value::Int(java::last_index_of(&s, &x, from) + 1))
        },
    );
    n.java(
        &format!("public java.lang.String {STRINGS}.replace(java.lang.String,java.lang.String,java.lang.String)"),
        |vm, c| {
            let (Some(s), Some(what), Some(with)) = (
                java::string(&c.self_),
                java::string(&c.args[0]),
                java::string(&c.args[1]),
            ) else {
                return Ok(c.self_.clone());
            };
            Ok(Value::str(&vm.native_replace_first(&s, &what, &with)?))
        },
    );
    n.java(
        &format!("public java.util.List {STRINGS}.split(java.lang.String,java.lang.String)"),
        |vm, c| split(vm, &c, Some(0)),
    );
    n.java(
        &format!("public java.util.List {STRINGS}.split(java.lang.String,java.lang.String,java.lang.Integer)"),
        |vm, c| {
            let limit = java::int(&c.args[1]);
            split(vm, &c, limit.map(|l| l as i32))
        },
    );

    // ---- EmfToolsLibrary ----
    n.java(
        &format!("public org.eclipse.emf.ecore.EDataType {EMF_TOOLS}.getDataTypeInstance(java.lang.String,java.lang.String)"),
        |vm, c| {
            let (Some(ns), Some(name)) = (java::string(&c.args[0]), java::string(&c.args[1]))
            else {
                return Ok(Value::Null);
            };
            // the generated package's `get<Name>()` accessor: data types only
            let meta = vm.store.meta();
            let Some(p) = meta.package_by_ns_uri(&ns) else {
                return Ok(Value::Null);
            };
            Ok(match meta.classifier_by_name(p, &name) {
                Some(d @ (Classifier::DataType(_) | Classifier::Enum(_))) => {
                    vm.native_classifier_object(d)
                }
                _ => Value::Null,
            })
        },
    );
    n.java(
        &format!("public java.lang.String {EMF_TOOLS}.getDetails(org.eclipse.emf.ecore.EAnnotation,java.lang.String)"),
        |vm, c| {
            if c.self_.is_null() {
                return Ok(Value::Null);
            }
            let key = java::string(&c.args[0]);
            let details = vm.native_get(&c.self_, "details")?;
            for e in vm.native_items(&details) {
                if java::string(&vm.native_get(&e, "key")?) == key {
                    return vm.native_get(&e, "value");
                }
            }
            Ok(Value::Null)
        },
    );
    n.java(
        &format!("public org.eclipse.emf.ecore.EObject {EMF_TOOLS}.asEObject(java.lang.Object)"),
        |_vm, c| {
            Ok(match &c.self_ {
                Value::Obj(_) => c.self_.clone(),
                _ => Value::Null,
            })
        },
    );
    n.java(
        &format!("public java.lang.Integer {EMF_TOOLS}.currentTimeMillis()"),
        |_vm, _c| {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Ok(Value::Int(ms as i32 as i64))
        },
    );
}

/// `Arrays.asList(self.split(regex[, limit]))`; `null` for a `null` argument.
fn split<S: ModelStore>(vm: &mut Vm<'_, S>, c: &NativeCall, limit: Option<i32>) -> NativeResult
where
    S::Obj: ObjHandle,
{
    let (Some(s), Some(re), Some(limit)) = (java::string(&c.self_), java::string(&c.args[0]), limit)
    else {
        return Ok(Value::Null);
    };
    let parts = vm.native_split(&s, &re, limit)?;
    let items = parts.iter().map(|p| Value::str(p)).collect();
    Ok(vm.native_collection(CollKind::Sequence, items))
}
