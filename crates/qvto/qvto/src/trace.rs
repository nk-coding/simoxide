//! Reading a `.qvtotrace` model (Eclipse QVTo's trace metamodel,
//! `http:///www.eclipse.org/m2m/qvt/operational/trace.ecore`) into the input trace of an
//! incremental update ([`qvto_vm::IncrementalTrace`]).

use qvto_emf::{Graph, ObjId};
use qvto_model::{ClassId, Datum, FeatureId, ModelStore};
use qvto_sema::ir::Direction;
use qvto_vm::{IncrementalTrace, TraceParam, TraceRecordIn, TraceValue};

pub const TRACE_NS_URI: &str = "http:///www.eclipse.org/m2m/qvt/operational/trace.ecore";

struct Features<'g> {
    g: &'g Graph,
}

impl Features<'_> {
    fn class(&self, name: &str) -> Option<ClassId> {
        let meta = self.g.meta();
        let p = meta.package_by_ns_uri(TRACE_NS_URI)?;
        meta.classes()
            .find(|&c| meta.class(c).package == p && &*meta.class(c).name == name)
    }
    fn feature(&self, class: &str, name: &str) -> Option<FeatureId> {
        let c = self.class(class)?;
        self.g.meta().feature_by_name(c, name)
    }
    fn obj(&self, o: ObjId, class: &str, name: &str) -> Option<ObjId> {
        self.g.get(o, self.feature(class, name)?).as_obj()
    }
    fn objs(&self, o: ObjId, class: &str, name: &str) -> Vec<ObjId> {
        let Some(f) = self.feature(class, name) else {
            return Vec::new();
        };
        (0..self.g.len(o, f))
            .filter_map(|i| self.g.get_at(o, f, i).as_obj())
            .collect()
    }
    fn string(&self, o: ObjId, class: &str, name: &str) -> Option<String> {
        match self.g.get(o, self.feature(class, name)?) {
            Datum::Str(s) => Some(s.to_string()),
            _ => None,
        }
    }
    fn kind(&self, o: ObjId) -> Direction {
        match self.feature("VarParameterValue", "kind").map(|f| self.g.get(o, f)) {
            Some(Datum::Enum(e, l)) => {
                match self.g.meta().enumeration(e).literals.get(l as usize).map(|x| x.name.as_ref()) {
                    Some("OUT") => Direction::Out,
                    Some("INOUT") => Direction::InOut,
                    _ => Direction::In,
                }
            }
            _ => Direction::In,
        }
    }

    /// `EValue`: a model element, a primitive value by type name, or a collection/tuple.
    fn value(&self, v: ObjId) -> TraceValue {
        if let Some(e) = self.obj(v, "EValue", "modelElement") {
            return if self.g.is_proxy(e) {
                TraceValue::Null
            } else {
                TraceValue::Obj(e.0)
            };
        }
        let ty = self.string(v, "EValue", "collectionType").unwrap_or_default();
        if let Some(p) = self.string(v, "EValue", "primitiveValue") {
            return match ty.as_str() {
                "Boolean" => TraceValue::Bool(p == "true"),
                "Integer" | "UnlimitedNatural" => {
                    p.parse().map(TraceValue::Int).unwrap_or(TraceValue::Null)
                }
                "Real" => p.parse().map(TraceValue::Real).unwrap_or(TraceValue::Null),
                _ => TraceValue::Str(p),
            };
        }
        let elems = self.objs(v, "EValue", "collection");
        match ty.as_str() {
            "Tuple" => TraceValue::Tuple(
                elems
                    .into_iter()
                    .map(|p| {
                        let name = self.string(p, "ETuplePartValue", "name").unwrap_or_default();
                        let val = self
                            .obj(p, "ETuplePartValue", "value")
                            .map(|x| self.value(x))
                            .unwrap_or(TraceValue::Null);
                        (name, val)
                    })
                    .collect(),
            ),
            "Dict" => {
                let mut kv = Vec::new();
                let parts: Vec<TraceValue> = elems
                    .into_iter()
                    .map(|p| {
                        self.obj(p, "ETuplePartValue", "value")
                            .map(|x| self.value(x))
                            .unwrap_or(TraceValue::Null)
                    })
                    .collect();
                for pair in parts.chunks(2) {
                    if let [k, v] = pair {
                        kv.push((k.clone(), v.clone()));
                    }
                }
                TraceValue::Dict(kv)
            }
            "" => TraceValue::Null,
            k => TraceValue::Coll(k.to_string(), elems.into_iter().map(|x| self.value(x)).collect()),
        }
    }

    fn param(&self, p: ObjId) -> TraceParam {
        TraceParam {
            name: self.string(p, "VarParameterValue", "name").unwrap_or_default(),
            kind: self.kind(p),
            type_name: self.string(p, "VarParameterValue", "type").unwrap_or_default(),
            value: self
                .obj(p, "VarParameterValue", "value")
                .map(|v| self.value(v))
                .unwrap_or(TraceValue::Null),
        }
    }
}

/// The records of the `Trace` roots among `roots` (proxies to the inputs and outputs of the
/// previous run must be resolved in the run's resource set).
pub fn read_incremental_trace(g: &Graph, roots: &[ObjId]) -> IncrementalTrace {
    let f = Features { g };
    let mut out = IncrementalTrace::default();
    let Some(trace) = f.class("Trace") else {
        return out;
    };
    for &t in roots {
        if g.class_of(t) != trace {
            continue;
        }
        for r in f.objs(t, "Trace", "traceRecords") {
            let Some(m) = f.obj(r, "TraceRecord", "mappingOperation") else {
                continue;
            };
            let context = f
                .obj(r, "TraceRecord", "context")
                .and_then(|c| f.obj(c, "EMappingContext", "context"))
                .map(|p| f.param(p));
            let params = f
                .obj(r, "TraceRecord", "parameters")
                .map(|ps| f.objs(ps, "EMappingParameters", "parameters"))
                .unwrap_or_default()
                .into_iter()
                .map(|p| f.param(p))
                .collect();
            let results = f
                .obj(r, "TraceRecord", "result")
                .map(|ps| f.objs(ps, "EMappingResults", "result"))
                .unwrap_or_default()
                .into_iter()
                .map(|p| f.param(p))
                .collect();
            out.records.push(TraceRecordIn {
                mapping: f.string(m, "EMappingOperation", "name").unwrap_or_default(),
                package: f.string(m, "EMappingOperation", "package").unwrap_or_default(),
                module: f.string(m, "EMappingOperation", "module").unwrap_or_default(),
                context,
                params,
                results,
            });
        }
    }
    out
}
