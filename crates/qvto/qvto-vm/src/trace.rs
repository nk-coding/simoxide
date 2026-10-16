//! Mapping traces (`Trace`, `TraceRecord`, `TraceUtil` of Eclipse) and the `resolve` family
//! (`QvtResolveUtil`).
//!
//! Records are kept in creation order with Eclipse's indexes: per mapping, per source key
//! (the context object, or the first `in`/`inout` parameter of a non-predefined type), per
//! target (the result value), and the lookup cache from `[self, non-out arguments...]` to the
//! latest record of a mapping. Keys compare with Java `equals` (identity for objects).

use crate::vm::{ObjHandle, Vm};
use qvto_bytecode::Site;
use qvto_model::ModelStore;
use qvto_runtime::coll::Coll;
use qvto_runtime::eq::{java_equals, java_hash};
use qvto_runtime::{HashEnv, Value};
use qvto_sema::ir::{Direction, OpId, ResolveKind};
use qvto_runtime::fxhash::FxHashMap as HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct TraceRecord {
    pub mapping: OpId,
    /// `self` of a contextual mapping.
    pub context: Option<Value>,
    pub params: Vec<(Direction, Value)>,
    /// The single result, or the tuple of several results.
    pub result: Option<Value>,
}

/// A map with Java `equals`/`hashCode` keys, iterating in first-insertion order (EMF's `EMap`).
#[derive(Clone, Debug, Default)]
pub struct ValueMap<V> {
    entries: Vec<(Value, V)>,
    index: HashMap<i32, Vec<usize>>,
}

impl<V> ValueMap<V> {
    pub fn get(&self, k: &Value, env: &dyn HashEnv) -> Option<&V> {
        let h = java_hash(k, env);
        self.index
            .get(&h)?
            .iter()
            .find(|&&i| java_equals(&self.entries[i].0, k, env))
            .map(|&i| &self.entries[i].1)
    }

    pub fn get_or_insert_with(
        &mut self,
        k: Value,
        env: &dyn HashEnv,
        f: impl FnOnce() -> V,
    ) -> &mut V {
        let h = java_hash(&k, env);
        let found = self.index.get(&h).and_then(|v| {
            v.iter()
                .copied()
                .find(|&i| java_equals(&self.entries[i].0, &k, env))
        });
        let i = match found {
            Some(i) => i,
            None => {
                self.entries.push((k, f()));
                let i = self.entries.len() - 1;
                self.index.entry(h).or_default().push(i);
                i
            }
        };
        &mut self.entries[i].1
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Traces {
    pub records: Vec<TraceRecord>,
    pub by_mapping: HashMap<OpId, Vec<usize>>,
    pub by_source: ValueMap<Vec<usize>>,
    pub by_target: ValueMap<Vec<usize>>,
    /// Per mapping: lookup key to the latest record with that key.
    pub cache: HashMap<OpId, ValueMap<usize>>,
}

impl Traces {
    pub fn len(&self) -> usize {
        self.records.len()
    }
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// The lookup key of a call: `self` alone for mappings without parameters, else the list
/// `[self, non-out arguments...]`.
pub(crate) fn trace_key(self_val: &Value, params: &[(Direction, Value)]) -> Value {
    if params.is_empty() {
        return self_val.clone();
    }
    let mut v = Vec::with_capacity(params.len() + 1);
    v.push(self_val.clone());
    for (d, x) in params {
        if *d != Direction::Out {
            v.push(x.clone());
        }
    }
    Value::Coll(Rc::new(Coll::Sequence(v)))
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// `TraceUtil.getTraceRecord`: the record of an earlier call with the same key whose
    /// result matches the current `result` value (if that is set).
    pub(crate) fn trace_lookup(
        &self,
        op: OpId,
        self_val: &Value,
        params: &[(Direction, Value)],
        result_now: &Value,
    ) -> Option<usize> {
        let key = trace_key(self_val, params);
        let rec = if key.is_null() {
            // no context and no parameters: Eclipse's linear scan matches the first record
            self.traces
                .by_mapping
                .get(&op)
                .and_then(|v| v.first().copied())
        } else {
            self.traces
                .cache
                .get(&op)
                .and_then(|m| m.get(&key, &self.env))
                .copied()
        }?;
        // checkResultMatch
        if !result_now.is_null() {
            match &self.traces.records[rec].result {
                Some(r) => {
                    if !ocl_identical(result_now, r, &self.env) {
                        return None;
                    }
                }
                None => return None,
            }
        }
        Some(rec)
    }

    /// `TraceUtil.addTraceRecord`.
    pub(crate) fn add_trace(
        &mut self,
        op: OpId,
        context: Option<Value>,
        params: Vec<(Direction, Value)>,
        result: Option<Value>,
    ) {
        let ir = &self.prog.ir;
        let o = &ir.operations[op.index()];
        let i = self.traces.records.len();
        // source key: the context, else the first in/inout parameter of a non-predefined type
        let source = match &context {
            Some(c) => Some(c.clone()),
            None => o.params.iter().zip(params.iter()).find_map(|(p, (_, v))| {
                let predefined = !matches!(
                    ir.types.get(p.ty),
                    qvto_types::Type::Class(_)
                        | qvto_types::Type::Enum(_)
                        | qvto_types::Type::DataType(_)
                        | qvto_types::Type::ModelType(_)
                        | qvto_types::Type::Module(_)
                );
                (p.direction != Direction::Out && !predefined).then(|| v.clone())
            }),
        };
        let env = &self.env;
        let params: Vec<(Direction, Value)> = params
            .into_iter()
            .map(|(d, v)| {
                let v = if d == Direction::In {
                    shallow_clone(v, env)
                } else {
                    v
                };
                (d, v)
            })
            .collect();
        self.traces.by_mapping.entry(op).or_default().push(i);
        if let Some(s) = source {
            self.traces
                .by_source
                .get_or_insert_with(s, env, Vec::new)
                .push(i);
        }
        if let Some(r) = &result {
            self.traces
                .by_target
                .get_or_insert_with(r.clone(), env, Vec::new)
                .push(i);
        }
        let self_val = context.clone().unwrap_or(Value::Null);
        let key = trace_key(&self_val, &params);
        if !key.is_null() {
            *self
                .traces
                .cache
                .entry(op)
                .or_default()
                .get_or_insert_with(key, env, || i) = i;
        }
        self.traces.records.push(TraceRecord {
            mapping: op,
            context,
            params,
            result,
        });
    }

    /// The candidates of a `resolve` expression in record order: `Sequence` for the plain and
    /// inverse kinds, the first one or null for the `one` kinds.
    pub(crate) fn resolve(&self, site: &Site, source: Value) -> Value {
        let Site::Resolve {
            kind,
            has_source,
            source_is_coll,
            mapping,
            target_ty,
        } = *site
        else {
            unreachable!()
        };
        let recs = self.resolve_records(kind, has_source, source_is_coll, mapping, &source);
        let mut out = Vec::new();
        for i in recs {
            let r = &self.traces.records[i];
            let cand = if kind.is_inverse() {
                r.context.clone()
            } else {
                r.result.clone()
            };
            let Some(c) = cand else { continue };
            if let Some(t) = target_ty
                && (c.is_undefined() || !self.is_kind_of(&c, t))
            {
                continue;
            }
            if kind.is_one() {
                return c;
            }
            out.push(c);
        }
        if kind.is_one() {
            Value::Null
        } else {
            Value::Coll(Rc::new(Coll::Sequence(out)))
        }
    }

    pub(crate) fn resolve_records(
        &self,
        kind: ResolveKind,
        has_source: bool,
        source_is_coll: bool,
        mapping: Option<OpId>,
        source: &Value,
    ) -> Vec<usize> {
        let mapping = mapping.map(|m| self.overriding(m));
        if let Some(m) = mapping
            && !has_source
        {
            return self.traces.by_mapping.get(&m).cloned().unwrap_or_default();
        }
        let map = if kind.is_inverse() {
            &self.traces.by_target
        } else {
            &self.traces.by_source
        };
        let mut recs: Vec<usize> = Vec::new();
        if !has_source {
            for v in map.values() {
                recs.extend(v.iter().copied());
            }
        } else if source_is_coll
            && matches!(source, Value::Coll(_) | Value::List(_) | Value::Dict(_))
        {
            for x in crate::vm::Iter::of(source).items.iter() {
                if let Some(v) = map.get(x, &self.env) {
                    recs.extend(v.iter().copied());
                }
            }
        } else if let Some(v) = map.get(source, &self.env) {
            recs.extend(v.iter().copied());
        }
        if let Some(m) = mapping {
            recs.retain(|&i| self.traces.records[i].mapping == m);
        }
        recs
    }
}

/// `isOclEqual` of trace matching: identity, both undefined, or Java `equals`.
fn ocl_identical(a: &Value, b: &Value, env: &dyn HashEnv) -> bool {
    if a.is_undefined() {
        return b.is_undefined();
    }
    if b.is_null() {
        return false;
    }
    java_equals(a, b, env)
}

/// IN `List`/`Dict` parameters are shallow copies in the record (`cloneOclObject`).
fn shallow_clone(v: Value, env: &dyn HashEnv) -> Value {
    match &v {
        Value::List(l) => Value::List(Rc::new(std::cell::RefCell::new(l.borrow().clone()))),
        Value::Dict(d) => Value::Dict(Rc::new(std::cell::RefCell::new(d.borrow().copy(env)))),
        _ => v,
    }
}
