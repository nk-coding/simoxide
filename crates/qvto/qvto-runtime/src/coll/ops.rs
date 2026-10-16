//! OCL collection operations as classic Eclipse OCL (`org.eclipse.ocl.util.CollectionUtil`) and
//! QVTo's overriding handlers (`org.eclipse.m2m.internal.qvt.oml.stdlib.*TypeOperations`)
//! implement them: which Java class each result has, in which order it is filled, and when the
//! source itself is returned. Ported from the EPL-2.0 sources named on each function.
//!
//! Operands are values; QVTo's `List` (`MutableListImpl`, a `java.util.List`) and `Dict`
//! (`DictionaryImpl`, a `java.util.Collection` of its values) are accepted where Eclipse
//! accepts any `Collection`. A Java exception in Eclipse (index out of bounds, class cast,
//! null pointer) is [`Value::Invalid`] here, as the evaluator turns it into `invalid`.
//!
//! Operations take their collection operands by value: when the caller holds the only
//! reference and the Java copy constructor would rebuild an identical table, the result reuses
//! the source's storage (copy on write).

use super::{Coll, CollKind, Index};
use crate::dict::List;
use crate::eq::{java_equals, ocl_equal};
use crate::identity::HashEnv;
use crate::java::hashmap::{JavaHashMap, hash_map_capacity_for, table_size_for};
use crate::value::{JNum, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// The Java class of a collection operand.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum JClass {
    Coll(CollKind),
    /// `MutableListImpl` (a `java.util.List`).
    MutableList,
    /// `DictionaryImpl` (a `java.util.Collection`, neither `List` nor `Set`).
    Dict,
}

impl JClass {
    fn is_set(self) -> bool {
        matches!(self, JClass::Coll(k) if k.is_set())
    }
    fn is_lhs(self) -> bool {
        self == JClass::Coll(CollKind::OrderedSet)
    }
    fn is_bag(self) -> bool {
        self == JClass::Coll(CollKind::Bag)
    }
    fn is_list(self) -> bool {
        matches!(self, JClass::Coll(CollKind::Sequence) | JClass::MutableList)
    }
}

/// A collection operand: its Java class and its elements as a [`Coll`] (a `List` or `Dict` is
/// viewed as a sequence of its elements, resp. values).
pub struct Operand {
    pub class: JClass,
    pub coll: Rc<Coll>,
    /// The original value (returned by operations that return their source).
    pub value: Value,
}

impl Operand {
    pub fn of(v: Value) -> Option<Operand> {
        match &v {
            Value::Coll(c) => Some(Operand {
                class: JClass::Coll(c.kind()),
                coll: c.clone(),
                value: v,
            }),
            Value::List(l) => {
                let c = Rc::new(Coll::Sequence(l.borrow().items.clone()));
                Some(Operand {
                    class: JClass::MutableList,
                    coll: c,
                    value: v,
                })
            }
            Value::Dict(d) => {
                let c = Rc::new(Coll::Sequence(d.borrow().values()));
                Some(Operand {
                    class: JClass::Dict,
                    coll: c,
                    value: v,
                })
            }
            _ => None,
        }
    }
    fn len(&self) -> usize {
        self.coll.len()
    }
    /// Drops the original value so that the storage can be reused if unique.
    fn into_coll(self) -> Rc<Coll> {
        drop(self.value);
        self.coll
    }
}

/// Default `HashMap` capacity after `n` distinct insertions (`new HashMap()`, growing).
fn default_growth_capacity(n: usize) -> usize {
    let mut cap = 16usize;
    while n > cap / 4 * 3 {
        cap *= 2;
    }
    cap
}

fn plain<V>(m: &JavaHashMap<Value, V>) -> bool {
    !m.has_tree_bins() && m.max_bin_len() < 8
}

/// True if `createNewCollection(c.kind(), c)` rebuilds exactly `c`'s storage, so a memory
/// copy (or reuse) gives the same iteration order as Java's copy constructor.
fn copy_equivalent(c: &Coll) -> bool {
    match c {
        Coll::Sequence(_) | Coll::OrderedSet(_) => true,
        Coll::Set(Index::Linked(_))
        | Coll::Bag(super::Bag {
            index: Index::Linked(_),
            ..
        }) => true,
        Coll::Set(Index::Hash(m)) => {
            let want = table_size_for(hash_map_capacity_for(m.len().max(12)));
            (m.capacity() == want || (m.capacity() == 0 && want == 16)) && plain(m)
        }
        Coll::Bag(super::Bag {
            index: Index::Hash(m),
            ..
        }) => {
            (m.capacity() == default_growth_capacity(m.len())
                || (m.capacity() == 0 && m.is_empty()))
                && plain(m)
        }
    }
}

/// `createNewCollection(kind, c)`, reusing `c`'s storage when that is indistinguishable.
fn copy_as(kind: CollKind, c: Rc<Coll>, env: &dyn HashEnv) -> Coll {
    if c.kind() == kind && copy_equivalent(&c) {
        return Rc::try_unwrap(c).unwrap_or_else(|rc| (*rc).clone());
    }
    Coll::copy_of(kind, &c, env)
}

fn list_value(c: Coll) -> Value {
    let items = match c {
        Coll::Sequence(v) => v,
        other => other.to_vec(),
    };
    Value::List(Rc::new(RefCell::new(List::from_vec(items))))
}

/// `Utils.createList(result)` when the source was a `MutableList` (QVTo re-wraps results).
fn rewrap(source: JClass, result: Value) -> Value {
    if source == JClass::MutableList
        && let Value::Coll(c) = result
    {
        return list_value(Rc::try_unwrap(c).unwrap_or_else(|rc| (*rc).clone()));
    }
    result
}

// ------------------------------------------------------------------ CollectionUtil

/// `CollectionUtil.includes` (Java `contains`).
pub fn includes(c: &Coll, v: &Value, env: &dyn HashEnv) -> bool {
    c.contains(v, env)
}

/// `CollectionUtil.count` (OCL equality, so `Sequence{1, 1.0}->count(1) = 2`).
pub fn count(c: &Coll, v: &Value, env: &dyn HashEnv) -> usize {
    c.iter().filter(|e| ocl_equal(e, v, env)).count()
}

/// `CollectionUtil.includesAll`.
pub fn includes_all(c: &Coll, d: &Coll, env: &dyn HashEnv) -> bool {
    d.iter().all(|e| c.contains(e, env))
}

/// `CollectionUtil.excludesAll`.
pub fn excludes_all(c: &Coll, d: &Coll, env: &dyn HashEnv) -> bool {
    !d.iter().any(|e| c.contains(e, env))
}

/// `CollectionUtil.including`: kind-preserving copy, then `add` (a `Dict` source gives a
/// sequence).
pub fn including(src: Value, v: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let kind = match op.class {
        JClass::Coll(k) => k,
        _ => CollKind::Sequence,
    };
    let mut r = copy_as(kind, op.into_coll(), env);
    r.add(v, env);
    r.into()
}

/// `CollectionUtil.excluding`: copy, then remove (a sequence drops every occurrence; a
/// `Dict` source fails).
pub fn excluding(src: Value, v: &Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let kind = match op.class {
        JClass::Coll(k) => k,
        JClass::MutableList => CollKind::Sequence,
        JClass::Dict => return Value::Invalid,
    };
    let mut r = copy_as(kind, op.into_coll(), env);
    if let Coll::Sequence(s) = &mut r {
        s.retain(|e| !java_equals(e, v, env));
    } else {
        r.remove(v, env);
    }
    r.into()
}

/// `CollectionUtil.union`.
pub fn union(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let (Some(a), Some(b)) = (Operand::of(a), Operand::of(b)) else {
        return Value::Invalid;
    };
    let either_bag = a.class.is_bag() || b.class.is_bag();
    if a.len() == 0 {
        return if either_bag {
            Coll::copy_of(CollKind::Bag, &b.coll, env).into()
        } else {
            create_new_collection(&b, env).into()
        };
    }
    if b.len() == 0 {
        return if either_bag {
            Coll::copy_of(CollKind::Bag, &a.coll, env).into()
        } else {
            create_new_collection(&a, env).into()
        };
    }
    let kind = if either_bag {
        CollKind::Bag
    } else if a.class.is_list() || b.class.is_list() {
        CollKind::Sequence
    } else {
        CollKind::Set
    };
    let bc = b.coll.clone();
    let mut r = copy_as(kind, a.into_coll(), env);
    r.add_all(&bc, env);
    r.into()
}

/// `CollectionUtil.createNewCollection(c)`: copy of the same kind (anything that is not a
/// Bag, OrderedSet or Set becomes a Sequence).
fn create_new_collection(c: &Operand, env: &dyn HashEnv) -> Coll {
    let kind = match c.class {
        JClass::Coll(k) => k,
        _ => CollKind::Sequence,
    };
    Coll::copy_of(kind, &c.coll, env)
}

/// `CollectionUtil.intersection`. An empty operand gives an empty Set (`Collections.emptySet()`)
/// if either operand is a set, else an empty Bag. Iterates the smaller operand (the source on
/// a tie), so a Bag result has the multiplicities of the smaller bag.
pub fn intersection(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let (Some(a), Some(b)) = (Operand::of(a), Operand::of(b)) else {
        return Value::Invalid;
    };
    let set = a.class.is_set() || b.class.is_set();
    let mut r = if set {
        Coll::new_set(env)
    } else {
        Coll::new_bag(env)
    };
    if a.len() == 0 || b.len() == 0 {
        return r.into();
    }
    let (small, large) = if a.len() > b.len() {
        (&b.coll, &a.coll)
    } else {
        (&a.coll, &b.coll)
    };
    for e in small.iter() {
        if large.contains(e, env) {
            r.add(e.clone(), env);
        }
    }
    r.into()
}

/// `CollectionUtil.flatten`: looks only at the first element; flattens one level while it is
/// a collection (Bag stays Bag, any Set becomes a `HashSet`, else Sequence).
pub fn flatten(src: Value, env: &dyn HashEnv) -> Value {
    let Some(mut cur) = Operand::of(src) else {
        return Value::Invalid;
    };
    loop {
        let first = match cur.coll.iter().next() {
            None => break,
            Some(f) => f.clone(),
        };
        if Operand::of(first).is_none() {
            break;
        }
        let kind = if cur.class.is_bag() {
            CollKind::Bag
        } else if cur.class.is_set() {
            CollKind::Set
        } else {
            CollKind::Sequence
        };
        let mut r = Coll::new_of(kind, env);
        for e in cur.coll.iter() {
            match Operand::of(e.clone()) {
                Some(inner) => {
                    r.add_all(&inner.coll, env);
                }
                // ClassCastException on a non-collection element
                None => return Value::Invalid,
            }
        }
        let v: Value = r.into();
        cur = Operand::of(v).expect("collection");
    }
    cur.value
}

/// `CollectionUtil.minus`: `new HashSet(self).removeAll(set)`.
pub fn minus(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let (Some(a), Some(b)) = (Operand::of(a), Operand::of(b)) else {
        return Value::Invalid;
    };
    let mut r = copy_as(CollKind::Set, a.into_coll(), env);
    remove_all(&mut r, &b.coll, env);
    r.into()
}

/// `AbstractSet.removeAll(c)`: removes `c`'s elements one by one if the set is larger,
/// else iterates the set and removes through the iterator.
pub fn remove_all(set: &mut Coll, c: &Coll, env: &dyn HashEnv) -> bool {
    if set.len() > c.len() {
        let mut changed = false;
        for e in c.iter() {
            changed |= set.remove(e, env);
        }
        changed
    } else {
        match set {
            Coll::Set(ix) => ix.retain(|e, _| !c.contains(e, env), env),
            Coll::OrderedSet(ix) => ix.retain(|e, _| !c.contains(e, env), env),
            _ => unreachable!("removeAll on a set"),
        }
    }
}

/// `CollectionUtil.symmetricDifference`.
pub fn symmetric_difference(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let (Some(a), Some(b)) = (Operand::of(a), Operand::of(b)) else {
        return Value::Invalid;
    };
    let mut r = copy_as(CollKind::Set, a.into_coll(), env);
    for e in b.coll.iter() {
        if r.contains(e, env) {
            r.remove(e, env);
        } else {
            r.add(e.clone(), env);
        }
    }
    r.into()
}

/// `CollectionUtil.asSet`: a `HashSet` is returned as is.
pub fn as_set(src: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    if op.class == JClass::Coll(CollKind::Set) {
        return op.value;
    }
    Coll::copy_of(CollKind::Set, &op.coll, env).into()
}

/// `CollectionUtil.asBag`.
pub fn as_bag(src: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    if op.class.is_bag() {
        return op.value;
    }
    Coll::copy_of(CollKind::Bag, &op.coll, env).into()
}

/// `CollectionUtil.asSequence`: any `java.util.List` (also a QVTo `List`) is returned as is.
pub fn as_sequence(src: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    if op.class.is_list() {
        return op.value;
    }
    Coll::copy_of(CollKind::Sequence, &op.coll, env).into()
}

/// `CollectionUtil.asOrderedSet`.
pub fn as_ordered_set(src: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    if op.class.is_lhs() {
        return op.value;
    }
    Coll::copy_of(CollKind::OrderedSet, &op.coll, env).into()
}

/// `CollectionUtil.append`: an OrderedSet moves an existing element to the end; anything else
/// becomes a Sequence.
pub fn append(src: Value, v: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let lhs = op.class.is_lhs();
    let mut r = copy_as(
        if lhs {
            CollKind::OrderedSet
        } else {
            CollKind::Sequence
        },
        op.into_coll(),
        env,
    );
    if lhs {
        r.remove(&v, env);
    }
    r.add(v, env);
    r.into()
}

/// `CollectionUtil.prepend`.
pub fn prepend(src: Value, v: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let mut r = if op.class.is_lhs() {
        Coll::new_ordered_set()
    } else {
        Coll::new_sequence()
    };
    r.add(v, env);
    r.add_all(&op.coll, env);
    r.into()
}

/// `CollectionUtil.insertAt` (1-based; `index` may be `size + 1`).
pub fn insert_at(src: Value, index: i32, v: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let i = index as i64 - 1;
    let n = op.len();
    if i < 0 || i > n as i64 {
        return Value::Invalid;
    }
    let i = i as usize;
    let mut r = if op.class.is_lhs() {
        Coll::new_ordered_set()
    } else {
        Coll::new_sequence()
    };
    for (cur, e) in op.coll.iter().enumerate() {
        if cur == i {
            r.add(v.clone(), env);
        }
        r.add(e.clone(), env);
    }
    if i == n {
        r.add(v, env);
    }
    r.into()
}

fn sub_range(n: usize, lower: i32, upper: i32) -> Option<(usize, usize)> {
    let (l, u) = (lower as i64 - 1, upper as i64 - 1);
    if l < 0 || u >= n as i64 || u < l {
        return None;
    }
    Some((l as usize, u as usize))
}

/// `CollectionUtil.subOrderedSet` (a non-OrderedSet source gives a Sequence).
pub fn sub_ordered_set(src: Value, lower: i32, upper: i32, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let Some((l, u)) = sub_range(op.len(), lower, upper) else {
        return Value::Invalid;
    };
    let mut r = if op.class.is_lhs() {
        Coll::new_ordered_set()
    } else {
        Coll::new_sequence()
    };
    for e in op.coll.iter().skip(l).take(u - l + 1) {
        r.add(e.clone(), env);
    }
    r.into()
}

/// `CollectionUtil.subSequence`.
pub fn sub_sequence(src: Value, lower: i32, upper: i32) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let Some((l, u)) = sub_range(op.len(), lower, upper) else {
        return Value::Invalid;
    };
    Coll::Sequence(op.coll.iter().skip(l).take(u - l + 1).cloned().collect()).into()
}

/// `CollectionUtil.at` (1-based, in iteration order also for Sets and Bags).
pub fn at(c: &Coll, index: i32) -> Value {
    let i = index as i64 - 1;
    if i < 0 || i >= c.len() as i64 {
        return Value::Invalid;
    }
    match c {
        Coll::Sequence(v) => v[i as usize].clone(),
        _ => c.iter().nth(i as usize).cloned().unwrap_or(Value::Invalid),
    }
}

/// `first()`: invalid when empty.
pub fn first(c: &Coll) -> Value {
    c.iter().next().cloned().unwrap_or(Value::Invalid)
}

/// `last()`: invalid when empty.
pub fn last(c: &Coll) -> Value {
    match c {
        Coll::Sequence(v) => v.last().cloned().unwrap_or(Value::Invalid),
        _ => c.iter().last().cloned().unwrap_or(Value::Invalid),
    }
}

/// `CollectionUtil.indexOf` (OCL equality); invalid if absent.
pub fn index_of(c: &Coll, v: &Value, env: &dyn HashEnv) -> Value {
    match c.iter().position(|e| ocl_equal(v, e, env)) {
        Some(i) => Value::Int(i as i64 + 1),
        None => Value::Invalid,
    }
}

/// Promotion of `NumberUtil.commonPrecisionNumber` for Integer/Long/Double operands.
fn common(a: JNum, b: JNum) -> (JNum, JNum) {
    let is_real = |n: JNum| matches!(n, JNum::Double(_) | JNum::Float(_));
    if is_real(a) || is_real(b) {
        (JNum::Double(a.as_f64()), JNum::Double(b.as_f64()))
    } else {
        (JNum::Long(a.as_i64()), JNum::Long(b.as_i64()))
    }
}

/// `CollectionUtil.sum`: `None` for an empty collection (the caller substitutes `0`/`0.0` by
/// element type, or null), invalid for a non-number element. Longs wrap.
pub fn sum(c: &Coll) -> Option<Value> {
    let mut acc: Option<JNum> = None;
    for e in c.iter() {
        let Some(n) = e.as_jnum() else {
            return Some(Value::Invalid);
        };
        acc = Some(match acc {
            None => n,
            Some(s) => match common(s, n) {
                (JNum::Double(x), JNum::Double(y)) => JNum::Double(x + y),
                (JNum::Long(x), JNum::Long(y)) => JNum::Long(x.wrapping_add(y)),
                _ => unreachable!(),
            },
        });
    }
    acc.map(|n| n.coerce().into_value())
}

fn extremum(c: &Coll, want: std::cmp::Ordering) -> Value {
    let mut acc: Option<JNum> = None;
    for e in c.iter() {
        let Some(n) = e.as_jnum() else {
            return Value::Invalid;
        };
        acc = Some(match acc {
            None => n,
            Some(m) => {
                let (m, n) = common(m, n);
                let ord = match (n, m) {
                    (JNum::Double(x), JNum::Double(y)) => crate::java::hash::double_compare(x, y),
                    (JNum::Long(x), JNum::Long(y)) => x.cmp(&y),
                    _ => unreachable!(),
                };
                if ord == want { n } else { m }
            }
        });
    }
    match acc {
        None => Value::Null,
        Some(n) => n.coerce().into_value(),
    }
}

/// `CollectionUtil.max`: null when empty; once a Real is seen the result is a Real.
pub fn max(c: &Coll) -> Value {
    extremum(c, std::cmp::Ordering::Greater)
}

/// `CollectionUtil.min`.
pub fn min(c: &Coll) -> Value {
    extremum(c, std::cmp::Ordering::Less)
}

// ------------------------------------------------------------------ QVTo handlers

/// QVTo's `Set/Bag/Sequence/List::including`: an OrderedSet source is first copied into a
/// `HashSet`; a `List` source gives a `List`.
pub fn qvto_including(src: Value, v: Value, env: &dyn HashEnv) -> Value {
    if v.is_invalid() {
        return Value::Invalid;
    }
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let class = op.class;
    let src = if class.is_lhs() {
        Coll::copy_of(CollKind::Set, &op.coll, env).into()
    } else {
        op.value
    };
    rewrap(class, including(src, v, env))
}

/// QVTo's `...::excluding` (same conversions as [`qvto_including`]).
pub fn qvto_excluding(src: Value, v: &Value, env: &dyn HashEnv) -> Value {
    if v.is_invalid() {
        return Value::Invalid;
    }
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let class = op.class;
    let src = if class.is_lhs() {
        Coll::copy_of(CollKind::Set, &op.coll, env).into()
    } else {
        op.value
    };
    rewrap(class, excluding(src, v, env))
}

/// QVTo's `...::union` (`List` source re-wrapped).
pub fn qvto_union(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let class = Operand::of(a.clone()).map(|o| o.class);
    let r = union(a, b, env);
    match class {
        Some(c) => rewrap(c, r),
        None => r,
    }
}

/// QVTo's `Set::-`: both operands must be `java.util.Set`s.
pub fn qvto_minus(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let sets = matches!(&a, Value::Coll(c) if c.kind().is_set())
        && matches!(&b, Value::Coll(c) if c.kind().is_set());
    if !sets {
        return Value::Invalid;
    }
    minus(a, b, env)
}

/// QVTo's `Set::symmetricDifference`.
pub fn qvto_symmetric_difference(a: Value, b: Value, env: &dyn HashEnv) -> Value {
    let sets = matches!(&a, Value::Coll(c) if c.kind().is_set())
        && matches!(&b, Value::Coll(c) if c.kind().is_set());
    if !sets {
        return Value::Invalid;
    }
    symmetric_difference(a, b, env)
}

/// QVTo's `Collection::flatten`: an OrderedSet is first copied into a `HashSet`; a `List`
/// source always gives a new `List`.
pub fn qvto_flatten(src: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let class = op.class;
    let s = if class.is_lhs() {
        Coll::copy_of(CollKind::Set, &op.coll, env).into()
    } else {
        op.value
    };
    let r = flatten(s, env);
    if class == JClass::MutableList {
        return match r {
            Value::List(l) => list_value(Coll::Sequence(l.borrow().items.clone())),
            Value::Coll(c) => list_value(Rc::try_unwrap(c).unwrap_or_else(|rc| (*rc).clone())),
            other => other,
        };
    }
    r
}

/// QVTo's `Collection::asSequence`: a `List` is copied into a new Sequence.
pub fn qvto_as_sequence(src: Value, env: &dyn HashEnv) -> Value {
    if let Value::List(l) = &src {
        return Coll::Sequence(l.borrow().items.clone()).into();
    }
    as_sequence(src, env)
}

/// QVTo's `Collection::asList`: a `List` is returned as is (aliasing), anything else copied.
pub fn qvto_as_list(src: Value) -> Value {
    match src {
        Value::List(_) => src,
        other => match Operand::of(other) {
            Some(op) => list_value(Coll::Sequence(op.coll.to_vec())),
            None => Value::Invalid,
        },
    }
}

/// QVTo's `OrderedSet/Sequence/List::append` (invalid argument: invalid).
pub fn qvto_append(src: Value, v: Value, env: &dyn HashEnv) -> Value {
    if v.is_invalid() {
        return Value::Invalid;
    }
    let class = Operand::of(src.clone()).map(|o| o.class);
    let r = append(src, v, env);
    match class {
        Some(c) => rewrap(c, r),
        None => r,
    }
}

/// QVTo's `...::prepend`.
pub fn qvto_prepend(src: Value, v: Value, env: &dyn HashEnv) -> Value {
    if v.is_invalid() {
        return Value::Invalid;
    }
    let class = Operand::of(src.clone()).map(|o| o.class);
    let r = prepend(src, v, env);
    match class {
        Some(c) => rewrap(c, r),
        None => r,
    }
}

/// QVTo's `...::insertAt(index, v)`.
pub fn qvto_insert_at(src: Value, index: i32, v: Value, env: &dyn HashEnv) -> Value {
    if v.is_invalid() {
        return Value::Invalid;
    }
    let class = Operand::of(src.clone()).map(|o| o.class);
    let r = insert_at(src, index, v, env);
    match class {
        Some(c) => rewrap(c, r),
        None => r,
    }
}

/// QVTo's `reverse`: List stays List, OrderedSet stays OrderedSet, else Sequence.
pub fn qvto_reverse(src: Value, env: &dyn HashEnv) -> Value {
    let Some(op) = Operand::of(src) else {
        return Value::Invalid;
    };
    let mut items = op.coll.to_vec();
    items.reverse();
    match op.class {
        JClass::MutableList => list_value(Coll::Sequence(items)),
        JClass::Coll(CollKind::OrderedSet) => {
            Coll::from_values(CollKind::OrderedSet, items, env).into()
        }
        _ => Coll::Sequence(items).into(),
    }
}
