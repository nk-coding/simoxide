//! The Eclipse QVTo test suite's lenient model comparison (`TreeComparator` with
//! `EmfObjectComparatorTreeNode`), as a verdict: equal or not.
//!
//! The suite turns each object into a node whose children are one node per non-empty,
//! non-derived reference (containments first); containment reference nodes have the contained
//! objects as children, the others refer to their targets. Children are sorted (stable) by the
//! node comparison (node kind, class name, then attribute values in `eAllAttributes` order), then
//! compared position by position. Non-containment reference targets are compared shallowly (class
//! name and attributes only), so a reference to a different object that looks the same passes.
//! Containment order does not matter (beyond the sort), many-valued attributes are compared as
//! lists. The tree-edit distance of the original is zero exactly when this recursion succeeds.
//!
//! Ported from `org.eclipse.m2m.tests.qvt.oml.api.framework.comparator` (EPL-2.0, see
//! `LICENSES`).

use crate::datatypes::{java_double_to_string, java_float_to_string};
use qvto_model::{Datum, FeatureId, FeatureKind, MetaRegistry, ModelStore};
use std::cmp::Ordering;

const UNWANTED: i64 = 1_000_000;

#[derive(Copy, Clone)]
enum Node<O> {
    Obj(O),
    /// A non-empty reference of an object.
    Ref(O, FeatureId),
}

/// Java `String.compareTo` (UTF-16 code units).
pub fn java_cmp_str(a: &str, b: &str) -> i64 {
    let mut x = a.encode_utf16();
    let mut y = b.encode_utf16();
    let (la, lb) = (a.encode_utf16().count() as i64, b.encode_utf16().count() as i64);
    loop {
        match (x.next(), y.next()) {
            (Some(p), Some(q)) if p != q => return p as i64 - q as i64,
            (Some(_), Some(_)) => {}
            _ => return la - lb,
        }
    }
}

/// `Object.toString()` of a value as the comparator sees it.
fn to_string<O: Copy>(meta: &MetaRegistry, d: &Datum<'_, O>) -> String {
    match d {
        Datum::Null => "null".into(),
        Datum::Bool(b) => b.to_string(),
        Datum::Int(i) => i.to_string(),
        Datum::Long(i) => i.to_string(),
        Datum::Short(i) => i.to_string(),
        Datum::Byte(i) => i.to_string(),
        Datum::Char(c) => char::from_u32(*c as u32).map(|c| c.to_string()).unwrap_or_default(),
        Datum::Double(x) => java_double_to_string(*x),
        Datum::Float(x) => java_float_to_string(*x),
        Datum::Str(s) | Datum::Other(s) => s.to_string(),
        Datum::Enum(e, i) => meta.enumeration(*e).literals[*i as usize].literal.to_string(),
        Datum::Obj(_) => "obj".into(),
    }
}

/// `compareAttributeValues` for single values: `compareNulls`, `equals`, `compareTo`, then
/// `toString` (enumeration literals are not `Comparable`).
fn cmp_values<A: Copy + PartialEq, B: Copy + PartialEq>(ma: &MetaRegistry, a: &Datum<'_, A>, mb: &MetaRegistry, b: &Datum<'_, B>) -> i64 {
    match (a, b) {
        (Datum::Null, Datum::Null) => return 0,
        (Datum::Null, _) => return -1,
        (_, Datum::Null) => return 1,
        _ => {}
    }
    let num = |x: i64, y: i64| (x > y) as i64 - (x < y) as i64;
    match (a, b) {
        (Datum::Str(x), Datum::Str(y)) | (Datum::Other(x), Datum::Other(y)) => java_cmp_str(x, y),
        (Datum::Bool(x), Datum::Bool(y)) => (*x as i64) - (*y as i64),
        (Datum::Int(x), Datum::Int(y)) => num(*x as i64, *y as i64),
        (Datum::Long(x), Datum::Long(y)) => num(*x, *y),
        (Datum::Short(x), Datum::Short(y)) => *x as i64 - *y as i64,
        (Datum::Byte(x), Datum::Byte(y)) => *x as i64 - *y as i64,
        (Datum::Char(x), Datum::Char(y)) => *x as i64 - *y as i64,
        (Datum::Double(x), Datum::Double(y)) => match x.total_cmp(y) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        },
        (Datum::Float(x), Datum::Float(y)) => match x.total_cmp(y) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        },
        _ => java_cmp_str(&to_string(ma, a), &to_string(mb, b)),
    }
}

struct Side<'a, S: ModelStore> {
    s: &'a S,
}

impl<'a, S: ModelStore> Side<'a, S> {
    fn class_name(&self, o: S::Obj) -> &'a str {
        &self.s.meta().class(self.s.class_of(o)).name
    }

    fn attrs(&self, o: S::Obj) -> Vec<FeatureId> {
        let meta = self.s.meta();
        meta.class(self.s.class_of(o))
            .all_features
            .iter()
            .copied()
            .filter(|f| meta.feature(*f).kind == FeatureKind::Attribute)
            .collect()
    }

    /// The comparator's view of an attribute value: a single value or the list's `toString`.
    fn attr_value(&self, o: S::Obj, f: FeatureId) -> AttrVal<'a, S::Obj> {
        let meta = self.s.meta();
        if meta.feature(f).is_many() {
            let items: Vec<String> = (0..self.s.len(o, f)).map(|i| to_string(meta, &self.s.get_at(o, f, i))).collect();
            let items_eq: Vec<Datum<'a, S::Obj>> = (0..self.s.len(o, f)).map(|i| self.s.get_at(o, f, i)).collect();
            AttrVal::List(items_eq, format!("[{}]", items.join(", ")))
        } else {
            AttrVal::One(self.s.get(o, f))
        }
    }

    fn values(&self, o: S::Obj, f: FeatureId) -> Vec<S::Obj> {
        if self.s.meta().feature(f).is_many() {
            (0..self.s.len(o, f)).filter_map(|i| self.s.get_at(o, f, i).as_obj()).collect()
        } else {
            self.s.get(o, f).as_obj().into_iter().collect()
        }
    }

    /// `getChildrenImpl` of an object node: its non-empty, non-derived references,
    /// containments first, in `eAllReferences` order.
    fn ref_nodes(&self, o: S::Obj) -> Vec<Node<S::Obj>> {
        let meta = self.s.meta();
        let refs: Vec<FeatureId> = meta
            .class(self.s.class_of(o))
            .all_features
            .iter()
            .copied()
            .filter(|f| meta.feature(*f).kind == FeatureKind::Reference && !meta.feature(*f).derived)
            .collect();
        let mut out = Vec::new();
        for containment in [true, false] {
            for &f in &refs {
                if meta.feature(f).containment != containment {
                    continue;
                }
                if !self.values(o, f).is_empty() {
                    out.push(Node::Ref(o, f));
                }
            }
        }
        out
    }
}

enum AttrVal<'a, O> {
    One(Datum<'a, O>),
    List(Vec<Datum<'a, O>>, String),
}

/// Compares two models (root lists) the way the Eclipse suite does (`ModelTestData`): same
/// number of roots and each pair of roots equal.
pub fn suite_equal<A: ModelStore, B: ModelStore>(sa: &A, ra: &[A::Obj], sb: &B, rb: &[B::Obj]) -> bool {
    let c = Cmp { a: Side { s: sa }, b: Side { s: sb } };
    ra.len() == rb.len() && ra.iter().zip(rb).all(|(x, y)| c.tree_equal(Node::Obj(*x), Node::Obj(*y)))
}

struct Cmp<'a, A: ModelStore, B: ModelStore> {
    a: Side<'a, A>,
    b: Side<'a, B>,
}

/// Node comparisons need both sides of the same store type for sorting; sorting happens within
/// one side, so a same-side comparison is provided by `cmp_same`.
fn cmp_same<S: ModelStore>(side: &Side<'_, S>, l: &Node<S::Obj>, r: &Node<S::Obj>) -> i64 {
    cmp_nodes(side, l, side, r)
}

/// `getChange(to).getCmp()`.
fn cmp_nodes<A: ModelStore, B: ModelStore>(sa: &Side<'_, A>, l: &Node<A::Obj>, sb: &Side<'_, B>, r: &Node<B::Obj>) -> i64 {
    match (l, r) {
        // NodeClassContentChange: "Emf(Containment|Object)ComparatorTreeNode" class names
        (Node::Ref(..), Node::Obj(_)) => java_cmp_str("EmfContainmentComparatorTreeNode", "EmfObjectComparatorTreeNode") * UNWANTED,
        (Node::Obj(_), Node::Ref(..)) => java_cmp_str("EmfObjectComparatorTreeNode", "EmfContainmentComparatorTreeNode") * UNWANTED,
        (Node::Ref(_, f), Node::Ref(_, g)) => {
            java_cmp_str(&sa.s.meta().feature(*f).name, &sb.s.meta().feature(*g).name) * UNWANTED
        }
        (Node::Obj(x), Node::Obj(y)) => {
            let c = java_cmp_str(sa.class_name(*x), sb.class_name(*y)) * UNWANTED;
            if c != 0 {
                return c;
            }
            let (ma, mb) = (sa.s.meta(), sb.s.meta());
            for f in sa.attrs(*x) {
                let name = &ma.feature(f).name;
                let Some(g) = mb.feature_by_name(sb.s.class_of(*y), name) else { continue };
                let c = match (sa.attr_value(*x, f), sb.attr_value(*y, g)) {
                    (AttrVal::One(p), AttrVal::One(q)) => cmp_values(ma, &p, mb, &q),
                    (AttrVal::List(p, ps), AttrVal::List(q, qs)) => {
                        let eq = p.len() == q.len() && p.iter().zip(&q).all(|(u, v)| cmp_values(ma, u, mb, v) == 0);
                        if eq { 0 } else { java_cmp_str(&ps, &qs) }
                    }
                    _ => 1,
                };
                if c != 0 {
                    return c;
                }
            }
            0
        }
    }
}

impl<A: ModelStore, B: ModelStore> Cmp<'_, A, B> {
    fn children_a(&self, n: &Node<A::Obj>) -> Vec<Node<A::Obj>> {
        let mut v = match n {
            Node::Obj(o) => self.a.ref_nodes(*o),
            Node::Ref(o, f) if self.a.s.meta().feature(*f).containment => self.a.values(*o, *f).into_iter().map(Node::Obj).collect(),
            Node::Ref(..) => Vec::new(),
        };
        v.sort_by(|x, y| cmp_same(&self.a, x, y).cmp(&0));
        v
    }
    fn children_b(&self, n: &Node<B::Obj>) -> Vec<Node<B::Obj>> {
        let mut v = match n {
            Node::Obj(o) => self.b.ref_nodes(*o),
            Node::Ref(o, f) if self.b.s.meta().feature(*f).containment => self.b.values(*o, *f).into_iter().map(Node::Obj).collect(),
            Node::Ref(..) => Vec::new(),
        };
        v.sort_by(|x, y| cmp_same(&self.b, x, y).cmp(&0));
        v
    }
    fn targets_a(&self, n: &Node<A::Obj>) -> Vec<Node<A::Obj>> {
        let mut v: Vec<Node<A::Obj>> = match n {
            Node::Ref(o, f) if !self.a.s.meta().feature(*f).containment => self.a.values(*o, *f).into_iter().map(Node::Obj).collect(),
            _ => Vec::new(),
        };
        v.sort_by(|x, y| cmp_same(&self.a, x, y).cmp(&0));
        v
    }
    fn targets_b(&self, n: &Node<B::Obj>) -> Vec<Node<B::Obj>> {
        let mut v: Vec<Node<B::Obj>> = match n {
            Node::Ref(o, f) if !self.b.s.meta().feature(*f).containment => self.b.values(*o, *f).into_iter().map(Node::Obj).collect(),
            _ => Vec::new(),
        };
        v.sort_by(|x, y| cmp_same(&self.b, x, y).cmp(&0));
        v
    }

    fn structure_equal(&self, l: Node<A::Obj>, r: Node<B::Obj>) -> bool {
        if cmp_nodes(&self.a, &l, &self.b, &r) != 0 {
            return false;
        }
        let (cl, cr) = (self.children_a(&l), self.children_b(&r));
        cl.len() == cr.len() && cl.into_iter().zip(cr).all(|(x, y)| self.structure_equal(x, y))
    }

    fn references_equal(&self, l: Node<A::Obj>, r: Node<B::Obj>) -> bool {
        let (tl, tr) = (self.targets_a(&l), self.targets_b(&r));
        if tl.len() != tr.len() {
            return false;
        }
        if !tl.iter().zip(&tr).all(|(x, y)| cmp_nodes(&self.a, x, &self.b, y) == 0) {
            return false;
        }
        let (cl, cr) = (self.children_a(&l), self.children_b(&r));
        cl.into_iter().zip(cr).all(|(x, y)| self.references_equal(x, y))
    }

    fn tree_equal(&self, l: Node<A::Obj>, r: Node<B::Obj>) -> bool {
        self.structure_equal(l, r) && self.references_equal(l, r)
    }
}
