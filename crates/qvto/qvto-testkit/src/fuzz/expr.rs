//! Random well-typed OCL/ImperativeOCL expressions over primitive values, collections, tuples,
//! `Dict`/`List` and the fuzz metamodel.
//!
//! Expressions are trees ([`E`]) so that a finding can be minimised: a node can be replaced by
//! one of its children of the same type or by a literal of its type. Types are tracked as far as
//! the generator needs them to stay well-typed; where Eclipse's result type is not obvious (the
//! kind of some collection results) the type is `Coll(Collection, T)`, which only feeds
//! positions that accept any collection.

use super::{Rng, quote};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ck {
    Set,
    OrderedSet,
    Bag,
    Sequence,
    /// Some collection kind (the generator does not know which).
    Collection,
}

impl Ck {
    pub fn name(self) -> &'static str {
        match self {
            Ck::Set => "Set",
            Ck::OrderedSet => "OrderedSet",
            Ck::Bag => "Bag",
            Ck::Sequence => "Sequence",
            Ck::Collection => "Collection",
        }
    }
    fn ordered(self) -> bool {
        matches!(self, Ck::OrderedSet | Ck::Sequence)
    }
    fn unique(self) -> bool {
        matches!(self, Ck::OrderedSet | Ck::Set)
    }
    const CONCRETE: [Ck; 4] = [Ck::Set, Ck::OrderedSet, Ck::Bag, Ck::Sequence];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cls {
    Named,
    Root,
    Node,
    Leaf,
    Item,
}

impl Cls {
    pub fn name(self) -> &'static str {
        match self {
            Cls::Named => "Named",
            Cls::Root => "Root",
            Cls::Node => "Node",
            Cls::Leaf => "Leaf",
            Cls::Item => "Item",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Ty {
    Bool,
    Int,
    Real,
    Str,
    /// `OclAny`
    Any,
    Obj(Cls),
    /// The enumeration `Kind`.
    Enum,
    Coll(Ck, Box<Ty>),
    List(Box<Ty>),
    Dict(Box<Ty>, Box<Ty>),
    /// A tuple `Tuple(a : Integer, b : String)` (the only tuple type generated besides
    /// `product`'s).
    Tuple,
}

impl Ty {
    pub fn coll(k: Ck, t: Ty) -> Ty {
        Ty::Coll(k, Box::new(t))
    }
    /// The type as QVTo source.
    pub fn src(&self) -> String {
        match self {
            Ty::Bool => "Boolean".into(),
            Ty::Int => "Integer".into(),
            Ty::Real => "Real".into(),
            Ty::Str => "String".into(),
            Ty::Any => "OclAny".into(),
            Ty::Obj(c) => format!("FZ::{}", c.name()),
            Ty::Enum => "FZ::Kind".into(),
            Ty::Coll(k, t) => format!("{}({})", k.name(), t.src()),
            Ty::List(t) => format!("List({})", t.src()),
            Ty::Dict(k, v) => format!("Dict({}, {})", k.src(), v.src()),
            Ty::Tuple => "Tuple(a : Integer, b : String)".into(),
        }
    }
    /// Whether values of type `self` can be used where `want` is expected (OCL conformance,
    /// conservatively).
    pub fn conforms(&self, want: &Ty) -> bool {
        if self == want {
            return true;
        }
        match (self, want) {
            (_, Ty::Any) => !matches!(self, Ty::Coll(..) | Ty::List(_) | Ty::Dict(..)),
            (Ty::Int, Ty::Real) => true,
            (Ty::Obj(a), Ty::Obj(b)) => sub_class(*a, *b),
            (Ty::Coll(k1, t1), Ty::Coll(k2, t2)) => {
                (k1 == k2 || *k2 == Ck::Collection) && t1.conforms(t2)
            }
            _ => false,
        }
    }
    fn is_coll(&self) -> bool {
        matches!(self, Ty::Coll(..))
    }
}

/// Collections, `List` and `Dict` take `->` for operations on themselves (`.` would collect).
fn is_many(t: &Ty) -> bool {
    matches!(t, Ty::Coll(..) | Ty::List(_) | Ty::Dict(..))
}

fn sub_class(a: Cls, b: Cls) -> bool {
    a == b
        || matches!(
            (a, b),
            (Cls::Leaf, Cls::Node) | (Cls::Node | Cls::Leaf | Cls::Root | Cls::Item, Cls::Named)
        )
}

/// An expression tree: `parts[0] kid[0] parts[1] kid[1] ... parts[n]`.
#[derive(Clone, Debug)]
pub struct E {
    pub parts: Vec<String>,
    pub kids: Vec<E>,
    pub ty: Ty,
    /// Variables this node binds for its kids (iterator variables, `let`).
    pub binds: Vec<String>,
    /// A leaf that is a variable reference.
    pub var: Option<String>,
}

impl E {
    fn leaf(text: impl Into<String>, ty: Ty) -> E {
        E {
            parts: vec![text.into()],
            kids: Vec::new(),
            ty,
            binds: Vec::new(),
            var: None,
        }
    }
    fn var(name: &str, ty: Ty) -> E {
        E {
            var: Some(name.to_string()),
            ..E::leaf(name, ty)
        }
    }
    /// From a template with `{}` holes.
    fn tpl(t: &str, kids: Vec<E>, ty: Ty) -> E {
        let parts: Vec<String> = t.split("{}").map(String::from).collect();
        assert_eq!(parts.len(), kids.len() + 1, "template {t}");
        E {
            parts,
            kids,
            ty,
            binds: Vec::new(),
            var: None,
        }
    }
    fn binding(mut self, vars: &[&str]) -> E {
        self.binds = vars.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn render(&self) -> String {
        let mut o = String::new();
        self.render_into(&mut o);
        o
    }
    fn render_into(&self, o: &mut String) {
        for (i, p) in self.parts.iter().enumerate() {
            o.push_str(p);
            if let Some(k) = self.kids.get(i) {
                k.render_into(o);
            }
        }
    }
    pub fn size(&self) -> usize {
        1 + self.kids.iter().map(E::size).sum::<usize>()
    }
    /// Free variables.
    pub fn free(&self) -> BTreeSet<String> {
        let mut s = BTreeSet::new();
        if let Some(v) = &self.var {
            s.insert(v.clone());
        }
        for k in &self.kids {
            s.extend(k.free());
        }
        for b in &self.binds {
            s.remove(b);
        }
        s
    }
    /// Simpler variants of this expression with the same type (for minimisation): each node
    /// replaced by a same-typed child that does not use the node's bound variables, or by the
    /// default literal of its type.
    pub fn reductions(&self) -> Vec<E> {
        let mut out = Vec::new();
        if let Some(lit) = default_literal(&self.ty)
            && lit.render() != self.render()
        {
            out.push(lit);
        }
        for k in &self.kids {
            if k.ty == self.ty && self.binds.iter().all(|b| !k.free().contains(b)) {
                out.push(k.clone());
            }
        }
        for (i, k) in self.kids.iter().enumerate() {
            for r in k.reductions() {
                let mut e = self.clone();
                e.kids[i] = r;
                out.push(e);
            }
        }
        out
    }
}

/// The simplest literal of a type, if there is one the minimiser may use.
pub fn default_literal(t: &Ty) -> Option<E> {
    Some(match t {
        Ty::Bool => E::leaf("true", Ty::Bool),
        Ty::Int => E::leaf("1", Ty::Int),
        Ty::Real => E::leaf("1.5", Ty::Real),
        Ty::Str => E::leaf("'a'", Ty::Str),
        Ty::Any => E::leaf("1", Ty::Any),
        Ty::Enum => E::leaf("FZ::Kind::A", Ty::Enum),
        Ty::Tuple => E::leaf("Tuple{a = 1, b = 'a'}", Ty::Tuple),
        _ => return None,
    })
}

const INTS: &[&str] = &[
    "0", "1", "2", "3", "5", "7", "10", "42", "100", "1024", "65536", "2147483647", "2147483648",
    "4294967296", "9223372036854775807", "123456789",
];
const NEG_INTS: &[&str] = &["-1", "-7", "-2147483648", "-2147483649", "-9223372036854775807"];
const REALS: &[&str] = &[
    "0.0", "1.0", "0.5", "2.5", "0.1", "0.3", "3.0", "1.0E10", "1.0E-5", "123456789.125",
    "1.0E300", "3.4028235E38", "1.0E-320", "2.0E9", "100.0", "1.25E7",
];
const STRS: &[&str] = &[
    "", "a", "abc", "Hello World", "  a  b  ", "x,y,,z", "123", "-42", "+7", "0x1F", "1.5",
    " 2.5 ", "1e3", "NaN", "Infinity", "true", "TRUE", "false", "abcabc", "aBc", "it's",
    "tab\there", "line\nbreak", "back\\slash", "\u{df}", "\u{130}", "\u{c4}x", "\u{65e5}\u{672c}",
    "a.b.c", "%s", "{0}", "$1", "_id9", "9id", "  ", "a b\tc", "\u{1F600}", "'q'", "\"dq\"",
    "2147483648", "007",
];
/// Regular expressions that both Java and the `regex` crate accept.
const REGEXES: &[&str] = &["a", "[a-c]+", ".*", "b|c", "\\s+", "^a", "(a)(b)", "x*", "[0-9]+", "\\.", ""];
const REPLACEMENTS: &[&str] = &["", "-", "$1", "<$0>", "\\$", "x"];

/// Variables of the generated `main` that every expression may use (name, type).
pub fn base_scope() -> Vec<(String, Ty)> {
    let v = |n: &str, t: Ty| (n.to_string(), t);
    vec![
        v("i0", Ty::Int),
        v("i1", Ty::Int),
        v("r0", Ty::Real),
        v("s0", Ty::Str),
        v("s1", Ty::Str),
        v("b0", Ty::Bool),
        v("root", Ty::Obj(Cls::Root)),
        v("n0", Ty::Obj(Cls::Node)),
        v("n1", Ty::Obj(Cls::Node)),
        v("ns", Ty::coll(Ck::OrderedSet, Ty::Obj(Cls::Node))),
        v("all", Ty::coll(Ck::Set, Ty::Obj(Cls::Node))),
        v("l0", Ty::List(Box::new(Ty::Int))),
        v("d0", Ty::Dict(Box::new(Ty::Str), Box::new(Ty::Int))),
        v("seq0", Ty::coll(Ck::Sequence, Ty::Int)),
        v("set0", Ty::coll(Ck::Set, Ty::Str)),
    ]
}

/// The declarations of [`base_scope`] (statements of `main`, `m` the input model).
pub const BASE_DECLS: &str = "\
\tvar i0 : Integer := 7;
\tvar i1 : Integer := -3;
\tvar r0 : Real := 2.5;
\tvar s0 : String := 'Hello World';
\tvar s1 : String := '';
\tvar b0 : Boolean := true;
\tvar root : FZ::Root := m.rootObjects()![FZ::Root];
\tvar ns : OrderedSet(FZ::Node) := root.nodes;
\tvar n0 : FZ::Node := ns->first();
\tvar n1 : FZ::Node := ns->last();
\tvar all : Set(FZ::Node) := m.objectsOfType(FZ::Node);
\tvar l0 : List(Integer) := List{3, 1, 2};
\tvar d0 : Dict(String, Integer) := Dict{'a' = 1, 'b' = 2};
\tvar seq0 : Sequence(Integer) := Sequence{4, 2, 2, 9};
\tvar set0 : Set(String) := Set{'x', 'y', 'z'};
";

pub struct Gen<'r> {
    pub rng: &'r mut Rng,
    pub scope: Vec<(String, Ty)>,
    fresh: u32,
    /// Probability (percent) of `null`/`invalid` leaves.
    pub undefined_pct: u32,
}

impl<'r> Gen<'r> {
    pub fn new(rng: &'r mut Rng, scope: Vec<(String, Ty)>) -> Gen<'r> {
        Gen {
            rng,
            scope,
            fresh: 0,
            undefined_pct: 3,
        }
    }

    fn fresh(&mut self, p: &str) -> String {
        self.fresh += 1;
        format!("{p}{}", self.fresh)
    }

    /// A random type for values (element types of collections, `log` targets).
    pub fn any_type(&mut self, depth: u32) -> Ty {
        let w = if depth > 0 {
            [30, 12, 18, 6, 12, 4, 4, 8, 3, 3]
        } else {
            [30, 12, 18, 6, 12, 4, 4, 0, 0, 0]
        };
        match self.rng.weighted(&w) {
            0 => Ty::Int,
            1 => Ty::Real,
            2 => Ty::Str,
            3 => Ty::Bool,
            4 => Ty::Obj(*self.rng.pick(&[Cls::Node, Cls::Node, Cls::Leaf, Cls::Item])),
            5 => Ty::Enum,
            6 => Ty::Tuple,
            7 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let t = self.elem_type(depth - 1);
                Ty::coll(k, t)
            }
            8 => Ty::List(Box::new(self.elem_type(0))),
            _ => Ty::Any,
        }
    }

    fn elem_type(&mut self, depth: u32) -> Ty {
        match self.rng.weighted(&[35, 10, 22, 5, 15, 4, 4, 5]) {
            0 => Ty::Int,
            1 => Ty::Real,
            2 => Ty::Str,
            3 => Ty::Bool,
            4 => Ty::Obj(Cls::Node),
            5 => Ty::Enum,
            6 => Ty::Tuple,
            _ if depth > 0 => Ty::coll(*self.rng.pick(&Ck::CONCRETE), Ty::Int),
            _ => Ty::Any,
        }
    }

    fn vars_of(&self, t: &Ty) -> Vec<(String, Ty)> {
        self.scope
            .iter()
            .filter(|(_, vt)| vt.conforms(t))
            .cloned()
            .collect()
    }

    fn with_var<T>(&mut self, name: &str, t: Ty, f: impl FnOnce(&mut Self) -> T) -> T {
        self.scope.push((name.to_string(), t));
        let r = f(self);
        self.scope.pop();
        r
    }

    /// An expression of type `t` (conforming to it) of at most `depth` levels.
    pub fn ex(&mut self, t: &Ty, depth: u32) -> E {
        if self.rng.pct(self.undefined_pct) && !matches!(t, Ty::Coll(..) | Ty::List(_) | Ty::Dict(..)) {
            // typed: `null`/`invalid` have no operations of their own (OclVoid)
            let u = if self.rng.pct(70) { "null" } else { "invalid" };
            return E::leaf(format!("{u}.oclAsType({})", t.src()), t.clone());
        }
        if depth == 0 || self.rng.pct(18) {
            return self.leaf(t);
        }
        let d = depth - 1;
        // generic forms first, sometimes
        match self.rng.below(20) {
            0 => {
                let c = self.ex(&Ty::Bool, d);
                let a = self.ex(t, d);
                let b = self.ex(t, d);
                return E::tpl("if {} then {} else {} endif", vec![c, a, b], t.clone());
            }
            1 if !t.is_coll() => {
                let vt = self.any_type(0);
                let v = self.fresh("v");
                let init = self.ex(&vt, d);
                let body = self.with_var(&v.clone(), vt.clone(), |g| g.ex(t, d));
                return E::tpl(
                    &format!("let {v} : {} = {{}} in {{}}", vt.src()),
                    vec![init, body],
                    t.clone(),
                )
                .binding(&[&v]);
            }
            2 => {
                // an element of a collection
                let k = *self.rng.pick(&[Ck::Sequence, Ck::OrderedSet]);
                let c = self.ex(&Ty::coll(k, t.clone()), d);
                return match self.rng.below(4) {
                    0 => E::tpl("{}->first()", vec![c], t.clone()),
                    1 => E::tpl("{}->last()", vec![c], t.clone()),
                    2 => {
                        let i = self.small_int(d);
                        E::tpl("{}->at({})", vec![c, i], t.clone())
                    }
                    _ => {
                        let x = self.fresh("x");
                        let body = self.with_var(&x.clone(), t.clone(), |g| g.ex(&Ty::Bool, d));
                        E::tpl(&format!("{{}}->any({x} | {{}})"), vec![c, body], t.clone())
                            .binding(&[&x])
                    }
                };
            }
            3 if matches!(t, Ty::Int | Ty::Real | Ty::Str | Ty::Bool) => {
                let v = self.ex(&Ty::Any, d);
                return E::tpl(&format!("{{}}.oclAsType({})", t.src()), vec![v], t.clone());
            }
            _ => {}
        }
        match t {
            Ty::Int => self.int(d),
            Ty::Real => self.real(d),
            Ty::Bool => self.boolean(d),
            Ty::Str => self.string(d),
            Ty::Any => {
                let u = self.any_type(1);
                if matches!(u, Ty::Coll(..) | Ty::List(_)) {
                    let e = self.ex(&Ty::Int, d);
                    E { ty: Ty::Any, ..e }
                } else {
                    let e = self.ex(&u, d);
                    E { ty: Ty::Any, ..e }
                }
            }
            Ty::Obj(c) => self.object(*c, d),
            Ty::Enum => self.enumeration(d),
            Ty::Tuple => {
                let a = self.ex(&Ty::Int, d);
                let b = self.ex(&Ty::Str, d);
                E::tpl("Tuple{a = {}, b = {}}", vec![a, b], Ty::Tuple)
            }
            Ty::Coll(k, e) => self.collection(*k, e, d),
            Ty::List(e) => self.list(e, d),
            Ty::Dict(k, v) => self.dict(k, v, d),
        }
    }

    fn small_int(&mut self, depth: u32) -> E {
        if depth > 0 && self.rng.pct(30) {
            return self.ex(&Ty::Int, depth.min(1));
        }
        E::leaf(self.rng.range(0, 4).to_string(), Ty::Int)
    }

    fn leaf(&mut self, t: &Ty) -> E {
        let vars = self.vars_of(t);
        if !vars.is_empty() && self.rng.pct(40) {
            let (n, vt) = self.rng.pick(&vars).clone();
            return E::var(&n, if vt == *t { vt } else { t.clone() });
        }
        match t {
            Ty::Int => {
                if self.rng.pct(15) {
                    E::leaf(format!("({})", self.rng.pick(NEG_INTS)), Ty::Int)
                } else {
                    E::leaf(self.rng.pick(INTS).to_string(), Ty::Int)
                }
            }
            Ty::Real => {
                let r = self.rng.pick(REALS).to_string();
                if self.rng.pct(15) {
                    E::leaf(format!("(-{r})"), Ty::Real)
                } else {
                    E::leaf(r, Ty::Real)
                }
            }
            Ty::Str => E::leaf(quote(self.rng.pick(STRS)), Ty::Str),
            Ty::Bool => E::leaf(if self.rng.pct(50) { "true" } else { "false" }, Ty::Bool),
            Ty::Any => {
                let u = self.rng.pick(&[Ty::Int, Ty::Real, Ty::Str, Ty::Bool, Ty::Enum]).clone();
                let e = self.leaf(&u);
                E { ty: Ty::Any, ..e }
            }
            Ty::Enum => E::leaf(format!("FZ::Kind::{}", self.rng.pick(&["A", "B", "C"])), Ty::Enum),
            Ty::Tuple => {
                let a = self.leaf(&Ty::Int);
                let b = self.leaf(&Ty::Str);
                E::tpl("Tuple{a = {}, b = {}}", vec![a, b], Ty::Tuple)
            }
            Ty::Obj(c) => self.object_leaf(*c),
            Ty::Coll(k, e) => {
                let k = if *k == Ck::Collection { *self.rng.pick(&Ck::CONCRETE) } else { *k };
                if **e == Ty::Int && self.rng.pct(20) {
                    let a = self.rng.range(-1, 3);
                    let b = self.rng.range(0, 5);
                    return E::leaf(format!("{}{{{a}..{b}}}", k.name()), Ty::coll(k, Ty::Int));
                }
                let n = self.rng.range(1, 4) as usize;
                let kids: Vec<E> = (0..n).map(|_| self.leaf(e)).collect();
                let holes = vec!["{}"; n].join(", ");
                E::tpl(&format!("{}{{{holes}}}", k.name()), kids, Ty::coll(k, (**e).clone()))
            }
            Ty::List(e) => {
                let n = self.rng.range(1, 3) as usize;
                let kids: Vec<E> = (0..n).map(|_| self.leaf(e)).collect();
                let holes = vec!["{}"; n].join(", ");
                E::tpl(&format!("List{{{holes}}}"), kids, t.clone())
            }
            Ty::Dict(k, v) => {
                let n = self.rng.range(1, 3) as usize;
                let mut kids = Vec::new();
                // keys: literals only (`literalSimpleCS`)
                for _ in 0..n {
                    let key = loop {
                        let e = self.leaf(k);
                        if e.var.is_none() {
                            break e;
                        }
                    };
                    kids.push(key);
                    kids.push(self.leaf(v));
                }
                let holes = vec!["{} = {}"; n].join(", ");
                E::tpl(&format!("Dict{{{holes}}}"), kids, t.clone())
            }
        }
    }

    fn object_leaf(&mut self, c: Cls) -> E {
        match c {
            Cls::Root => E::var("root", Ty::Obj(Cls::Root)),
            Cls::Node | Cls::Named => {
                let n = *self.rng.pick(&["n0", "n1"]);
                E::var(n, Ty::Obj(c))
            }
            Cls::Leaf => E::leaf(
                "ns->select(x | x.oclIsKindOf(FZ::Leaf))->any(true).oclAsType(FZ::Leaf)",
                Ty::Obj(Cls::Leaf),
            ),
            Cls::Item => E::leaf("root.items->first()", Ty::Obj(Cls::Item)),
        }
    }

    fn int(&mut self, d: u32) -> E {
        let int = Ty::Int;
        match self.rng.below(30) {
            0..=2 => {
                let op = *self.rng.pick(&["+", "-", "*"]);
                let a = self.ex(&int, d);
                let b = self.ex(&int, d);
                E::tpl(&format!("({{}} {op} {{}})"), vec![a, b], int)
            }
            3 => {
                let op = *self.rng.pick(&["div", "mod", "max", "min"]);
                let a = self.ex(&int, d);
                let b = self.ex(&int, d);
                E::tpl(&format!("{{}}.{op}({{}})"), vec![a, b], int)
            }
            4 => {
                let a = self.ex(&int, d);
                E::tpl(if self.rng.pct(50) { "-{}" } else { "{}.abs()" }, vec![a], int)
            }
            5 => {
                let r = self.ex(&Ty::Real, d);
                E::tpl(if self.rng.pct(50) { "{}.floor()" } else { "{}.round()" }, vec![r], int)
            }
            6 => {
                let s = self.ex(&Ty::Str, d);
                E::tpl(self.rng.pick(&["{}.size()", "{}.length()"]), vec![s], int)
            }
            7 => {
                let s = self.ex(&Ty::Str, d);
                let u = self.ex(&Ty::Str, d);
                let op = *self.rng.pick(&["indexOf", "find", "rfind", "lastIndexOf"]);
                E::tpl(&format!("{{}}.{op}({{}})"), vec![s, u], int)
            }
            8 => {
                let s = self.numeric_string(d);
                E::tpl(self.rng.pick(&["{}.toInteger()", "{}.asInteger()"]), vec![s], int)
            }
            9 | 10 => {
                let et = self.elem_type(1);
                let k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, et.clone()), d);
                if self.rng.pct(50) {
                    E::tpl("{}->size()", vec![c], int)
                } else {
                    let x = self.ex(&et, d);
                    E::tpl("{}->count({})", vec![c, x], int)
                }
            }
            11 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, Ty::Int), d);
                E::tpl(self.rng.pick(&["{}->sum()", "{}->max()", "{}->min()"]), vec![c], int)
            }
            12 => {
                let k = *self.rng.pick(&[Ck::Sequence, Ck::OrderedSet]);
                let et = self.elem_type(0);
                let c = self.ex(&Ty::coll(k, et.clone()), d);
                let x = self.ex(&et, d);
                E::tpl("{}->indexOf({})", vec![c, x], int)
            }
            13 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let et = self.elem_type(0);
                let c = self.ex(&Ty::coll(k, et.clone()), d);
                let x = self.fresh("x");
                let acc = self.fresh("acc");
                let init = self.ex(&int, d);
                let body = self.with_var(&x.clone(), et, |g| {
                    g.with_var(&acc.clone(), Ty::Int, |g| g.ex(&Ty::Int, d))
                });
                E::tpl(
                    &format!("{{}}->iterate({x}; {acc} : Integer = {{}} | {{}})"),
                    vec![c, init, body],
                    int,
                )
                .binding(&[&x, &acc])
            }
            14 | 15 => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                match self.rng.below(4) {
                    0 | 1 => E::tpl("{}.value", vec![o], int),
                    2 => E::tpl("{}.big", vec![o], int),
                    _ => E::tpl("{}.nums->size()", vec![o], int),
                }
            }
            16 => {
                let o = self.ex(&Ty::Obj(Cls::Item), d);
                E::tpl("{}.count", vec![o], int)
            }
            17 => {
                let o = self.ex(&Ty::Obj(Cls::Leaf), d);
                E::tpl("{}.size", vec![o], int)
            }
            18 => {
                let dct = self.ex(&Ty::Dict(Box::new(Ty::Str), Box::new(Ty::Int)), d);
                let k = self.ex(&Ty::Str, d);
                if self.rng.pct(70) {
                    E::tpl("{}->get({})", vec![dct, k], int)
                } else {
                    let dflt = self.ex(&int, d);
                    E::tpl("{}->defaultget({}, {})", vec![dct, k, dflt], int)
                }
            }
            19 => {
                let tp = self.ex(&Ty::Tuple, d);
                E::tpl("{}.a", vec![tp], int)
            }
            20 => {
                let l = self.ex(&Ty::List(Box::new(Ty::Int)), d);
                match self.rng.below(3) {
                    0 => E::tpl("{}->size()", vec![l], int),
                    1 => E::tpl("{}->at(1)", vec![l], int),
                    _ => {
                        let x = self.ex(&int, d);
                        E::tpl("{}->indexOf({})", vec![l, x], int)
                    }
                }
            }
            21 => {
                let e = self.ex(&Ty::Enum, d);
                E::tpl("{}.toString().size()", vec![e], int)
            }
            22 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, Ty::Obj(Cls::Node)), d);
                E::tpl("{}->collect(value)->sum()", vec![c], int)
            }
            _ => self.leaf(&int),
        }
    }

    fn numeric_string(&mut self, d: u32) -> E {
        if self.rng.pct(70) {
            let s = *self.rng.pick(&[
                "0", "12", "-3", "+5", " 7", "2147483647", "2147483648", "-2147483648", "1.5",
                "1e3", "0x10", "", "abc", "007", "1_000", "\u{0663}", "NaN", "-0", "1.0E10",
                " 2.5 ", "Infinity", "-Infinity", "1f", "1d", ".5", "5.",
            ]);
            E::leaf(quote(s), Ty::Str)
        } else {
            self.ex(&Ty::Str, d)
        }
    }

    fn real(&mut self, d: u32) -> E {
        let real = Ty::Real;
        match self.rng.below(16) {
            0..=2 => {
                let op = *self.rng.pick(&["+", "-", "*", "/"]);
                let (ta, tb) = match self.rng.below(3) {
                    0 => (Ty::Real, Ty::Real),
                    1 => (Ty::Int, Ty::Real),
                    _ => (Ty::Real, Ty::Int),
                };
                let a = self.ex(&ta, d);
                let b = self.ex(&tb, d);
                E::tpl(&format!("({{}} {op} {{}})"), vec![a, b], real)
            }
            3 => {
                let a = self.ex(&Ty::Int, d);
                let b = self.ex(&Ty::Int, d);
                E::tpl("({} / {})", vec![a, b], real)
            }
            4 => {
                let a = self.ex(&real, d);
                match self.rng.below(4) {
                    0 => E::tpl("-{}", vec![a], real),
                    1 => E::tpl("{}.abs()", vec![a], real),
                    _ => {
                        let b = self.ex(&real, d);
                        let op = *self.rng.pick(&["max", "min"]);
                        E::tpl(&format!("{{}}.{op}({{}})"), vec![a, b], real)
                    }
                }
            }
            5 => {
                let s = self.numeric_string(d);
                E::tpl(self.rng.pick(&["{}.toReal()", "{}.asFloat()"]), vec![s], real)
            }
            6 => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.weight", vec![o], real)
            }
            7 => {
                let o = self.ex(&Ty::Obj(Cls::Item), d);
                E::tpl("{}.ratio", vec![o], real)
            }
            8 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, real.clone()), d);
                E::tpl(self.rng.pick(&["{}->sum()", "{}->max()", "{}->min()"]), vec![c], real)
            }
            9..=11 => self.ex(&Ty::Int, d + 1).retype(real),
            _ => self.leaf(&real),
        }
    }

    fn boolean(&mut self, d: u32) -> E {
        let b = Ty::Bool;
        match self.rng.below(30) {
            0..=2 => {
                let op = *self.rng.pick(&["and", "or", "xor", "implies"]);
                let x = self.ex(&b, d);
                let y = self.ex(&b, d);
                E::tpl(&format!("({{}} {op} {{}})"), vec![x, y], b)
            }
            3 => {
                let x = self.ex(&b, d);
                E::tpl("(not {})", vec![x], b)
            }
            4..=6 => {
                let t = self.rng.pick(&[Ty::Int, Ty::Real, Ty::Str]).clone();
                let (ta, tb) = if t == Ty::Str {
                    (Ty::Str, Ty::Str)
                } else {
                    (
                        if self.rng.pct(50) { Ty::Int } else { Ty::Real },
                        if self.rng.pct(50) { Ty::Int } else { Ty::Real },
                    )
                };
                let op = *self.rng.pick(&["<", ">", "<=", ">=", "=", "<>"]);
                let x = self.ex(&ta, d);
                let y = self.ex(&tb, d);
                E::tpl(&format!("({{}} {op} {{}})"), vec![x, y], b)
            }
            7 | 8 => {
                let t = self.any_type(1);
                let op = *self.rng.pick(&["=", "<>"]);
                let x = self.ex(&t, d);
                let y = self.ex(&t, d);
                E::tpl(&format!("({{}} {op} {{}})"), vec![x, y], b)
            }
            9..=11 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let et = self.elem_type(1);
                let c = self.ex(&Ty::coll(k, et.clone()), d);
                match self.rng.below(5) {
                    0 | 1 => {
                        let x = self.ex(&et, d);
                        let op = *self.rng.pick(&["includes", "excludes"]);
                        E::tpl(&format!("{{}}->{op}({{}})"), vec![c, x], b)
                    }
                    2 => {
                        let k2 = *self.rng.pick(&Ck::CONCRETE);
                        let c2 = self.ex(&Ty::coll(k2, et), d);
                        let op = *self.rng.pick(&["includesAll", "excludesAll"]);
                        E::tpl(&format!("{{}}->{op}({{}})"), vec![c, c2], b)
                    }
                    _ => {
                        let op = *self.rng.pick(&["isEmpty", "notEmpty"]);
                        E::tpl(&format!("{{}}->{op}()"), vec![c], b)
                    }
                }
            }
            12..=14 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let et = self.elem_type(1);
                let c = self.ex(&Ty::coll(k, et.clone()), d);
                let x = self.fresh("x");
                let it = *self.rng.pick(&["forAll", "exists", "one", "isUnique"]);
                let body = if it == "isUnique" {
                    let kt = self.rng.pick(&[Ty::Int, Ty::Str, Ty::Bool]).clone();
                    self.with_var(&x.clone(), et, |g| g.ex(&kt, d))
                } else {
                    self.with_var(&x.clone(), et, |g| g.ex(&Ty::Bool, d))
                };
                E::tpl(&format!("{{}}->{it}({x} | {{}})"), vec![c, body], b).binding(&[&x])
            }
            15 | 16 => {
                let s = self.ex(&Ty::Str, d);
                let u = self.ex(&Ty::Str, d);
                let op = *self.rng.pick(&["startsWith", "endsWith", "equalsIgnoreCase"]);
                E::tpl(&format!("{{}}.{op}({{}})"), vec![s, u], b)
            }
            17 => {
                let s = self.ex(&Ty::Str, d);
                let re = quote(self.rng.pick(REGEXES));
                E::tpl(&format!("{{}}.matches({re})"), vec![s], b)
            }
            18 | 19 => {
                let t = self.any_type(1);
                let x = self.ex(&t, d);
                let op = *self.rng.pick(&["oclIsUndefined", "oclIsInvalid"]);
                let arrow = if is_many(&t) { "->" } else { "." };
                E::tpl(&format!("{{}}{arrow}{op}()"), vec![x], b)
            }
            20 | 21 => {
                let t = self.any_type(0);
                let x = self.ex(&t, d);
                let target = *self.rng.pick(&[
                    "Integer", "Real", "String", "Boolean", "OclAny", "FZ::Node", "FZ::Leaf",
                    "FZ::Named", "FZ::Kind", "OclVoid",
                ]);
                let op = *self.rng.pick(&["oclIsKindOf", "oclIsTypeOf"]);
                E::tpl(&format!("{{}}.{op}({target})"), vec![x], b)
            }
            22 => {
                let s = self.ex(&Ty::Str, d);
                E::tpl(self.rng.pick(&["{}.toBoolean()", "{}.asBoolean()"]), vec![s], b)
            }
            23 => {
                let dct = self.ex(&Ty::Dict(Box::new(Ty::Str), Box::new(Ty::Int)), d);
                let k = self.ex(&Ty::Str, d);
                E::tpl("{}->hasKey({})", vec![dct, k], b)
            }
            24 => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.flag", vec![o], b)
            }
            25 => {
                let e1 = self.ex(&Ty::Enum, d);
                let e2 = self.ex(&Ty::Enum, d);
                E::tpl("({} = {})", vec![e1, e2], b)
            }
            26 => {
                let o1 = self.ex(&Ty::Obj(Cls::Node), d);
                let o2 = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("({} = {})", vec![o1, o2], b)
            }
            _ => self.leaf(&b),
        }
    }

    fn string(&mut self, d: u32) -> E {
        let s = Ty::Str;
        match self.rng.below(32) {
            0..=2 => {
                let a = self.ex(&s, d);
                let c = self.ex(&s, d);
                E::tpl(
                    if self.rng.pct(70) { "({} + {})" } else { "{}.concat({})" },
                    vec![a, c],
                    s,
                )
            }
            3 | 4 => {
                let a = self.ex(&s, d);
                let i = self.small_int(d);
                let j = self.small_int(d);
                E::tpl("{}.substring({}, {})", vec![a, i, j], s)
            }
            5..=7 => {
                let a = self.ex(&s, d);
                let op = *self.rng.pick(&[
                    "toUpper", "toLower", "toUpperCase", "toLowerCase", "firstToUpper",
                    "lastToUpper", "trim", "normalizeSpace",
                ]);
                E::tpl(&format!("{{}}.{op}()"), vec![a], s)
            }
            8 => {
                let a = self.ex(&s, d);
                let i = self.small_int(d);
                E::tpl("{}.at({})", vec![a, i], s)
            }
            9 => {
                let a = self.ex(&s, d);
                let x = self.ex(&s, d);
                let y = self.ex(&s, d);
                E::tpl("{}.replace({}, {})", vec![a, x, y], s)
            }
            10 => {
                let a = self.ex(&s, d);
                let re = quote(self.rng.pick(REGEXES));
                let rp = quote(self.rng.pick(REPLACEMENTS));
                let op = *self.rng.pick(&["replaceAll", "replaceFirst"]);
                E::tpl(&format!("{{}}.{op}({re}, {rp})"), vec![a], s)
            }
            11 => {
                let a = self.ex(&s, d);
                let x = self.ex(&s, d);
                let op = *self.rng.pick(&["substringAfter", "substringBefore"]);
                E::tpl(&format!("{{}}.{op}({{}})"), vec![a, x], s)
            }
            12 => {
                let a = self.ex(&s, d);
                let q = quote(self.rng.pick(&["'", "\"", "x", ""]));
                let op = *self.rng.pick(&["quotify", "unquotify"]);
                E::tpl(&format!("{{}}.{op}({q})"), vec![a], s)
            }
            13..=15 => {
                let t = self.any_type(1);
                let x = self.ex(&t, d);
                // toString() only on non-collections; repr() on everything
                if is_many(&t) {
                    E::tpl("{}->repr()", vec![x], s)
                } else if self.rng.pct(50) {
                    E::tpl("{}.repr()", vec![x], s)
                } else {
                    E::tpl("{}.toString()", vec![x], s)
                }
            }
            16 => {
                let oc = *self.rng.pick(&[Cls::Node, Cls::Item, Cls::Root]);
                let o = self.ex(&Ty::Obj(oc), d);
                E::tpl("{}.name", vec![o], s)
            }
            17 => {
                let et = self.elem_type(0);
                let k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, et), d);
                let sep = quote(self.rng.pick(&[",", ", ", ""]));
                E::tpl(&format!("{{}}->asList()->joinfields({sep}, '[', ']')"), vec![c], s)
            }
            18 => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl(self.rng.pick(&["{}.metaClassName()", "{}.kind.toString()", "{}.tags->first()"]), vec![o], s)
            }
            19 => {
                let a = self.ex(&s, d);
                E::tpl("{}.addSuffixNumber()", vec![a], s)
            }
            20 => {
                let tp = self.ex(&Ty::Tuple, d);
                E::tpl("{}.b", vec![tp], s)
            }
            21 => {
                let k = *self.rng.pick(&[Ck::Sequence, Ck::OrderedSet]);
                let c = self.ex(&Ty::coll(k, s.clone()), d);
                E::tpl("{}->iterate(x; acc : String = '' | acc + x)", vec![c], s)
            }
            _ => self.leaf(&s),
        }
    }

    fn object(&mut self, c: Cls, d: u32) -> E {
        let t = Ty::Obj(c);
        match (c, self.rng.below(6)) {
            (Cls::Node | Cls::Named, 0) => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.parent", vec![o], t)
            }
            (Cls::Node | Cls::Named, 1) => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl(
                    self.rng.pick(&["{}.children->first()", "{}.refs->last()", "{}.refs->any(true)"]),
                    vec![o],
                    t,
                )
            }
            (Cls::Node | Cls::Named, 2) => {
                let o = self.ex(&Ty::Obj(Cls::Item), d);
                E::tpl("{}.owner", vec![o], t)
            }
            (Cls::Item, 0) => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.first", vec![o], t)
            }
            (Cls::Leaf, 0) => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.oclAsType(FZ::Leaf)", vec![o], t)
            }
            _ => self.object_leaf(c),
        }
    }

    fn enumeration(&mut self, d: u32) -> E {
        if self.rng.pct(50) {
            let o = self.ex(&Ty::Obj(Cls::Node), d);
            E::tpl("{}.kind", vec![o], Ty::Enum)
        } else {
            self.leaf(&Ty::Enum)
        }
    }

    fn collection(&mut self, k: Ck, e: &Ty, d: u32) -> E {
        // choose a concrete kind when any will do
        let want_any = k == Ck::Collection;
        let k = if want_any { *self.rng.pick(&Ck::CONCRETE) } else { k };
        let ty = Ty::coll(k, e.clone());
        let fits = |r: &Ty| r.conforms(&Ty::coll(if want_any { Ck::Collection } else { k }, e.clone()));
        for _ in 0..4 {
            let r = self.collection_op(k, e, d);
            if fits(&r.ty) {
                return if want_any { r } else { r.retype(ty.clone()) };
            }
        }
        self.leaf(&ty)
    }

    /// A collection expression whose type is `Coll(k, e)` or, where the result kind is not
    /// certain, `Coll(Collection, e)` (rejected by `collection` unless any kind will do).
    fn collection_op(&mut self, k: Ck, e: &Ty, d: u32) -> E {
        let et = e.clone();
        let ty = Ty::coll(k, et.clone());
        match self.rng.below(26) {
            0 => {
                // conversion from another kind
                let k2 = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k2, et), d);
                E::tpl(&format!("{{}}->as{}()", k.name()), vec![c], ty)
            }
            1 | 2 => {
                let c = self.ex(&ty, d);
                let x = self.ex(e, d);
                let op = *self.rng.pick(&["including", "excluding"]);
                let rk = if k == Ck::OrderedSet { Ck::Collection } else { k };
                E::tpl(&format!("{{}}->{op}({{}})"), vec![c, x], Ty::coll(rk, (*e).clone()))
            }
            3 | 4 => {
                // union / intersection / difference with Eclipse's result kinds
                let (k1, k2, op, rk) = *self.rng.pick(&[
                    (Ck::Set, Ck::Set, "union", Ck::Set),
                    (Ck::Set, Ck::Bag, "union", Ck::Bag),
                    (Ck::Bag, Ck::Set, "union", Ck::Bag),
                    (Ck::Bag, Ck::Bag, "union", Ck::Bag),
                    (Ck::Sequence, Ck::Sequence, "union", Ck::Sequence),
                    (Ck::OrderedSet, Ck::Set, "union", Ck::Collection),
                    (Ck::Set, Ck::Set, "intersection", Ck::Set),
                    (Ck::Set, Ck::Bag, "intersection", Ck::Set),
                    (Ck::Bag, Ck::Set, "intersection", Ck::Set),
                    (Ck::Bag, Ck::Bag, "intersection", Ck::Bag),
                    (Ck::Set, Ck::Set, "symmetricDifference", Ck::Set),
                    (Ck::Set, Ck::Set, "-", Ck::Set),
                ]);
                let a = self.ex(&Ty::coll(k1, et.clone()), d);
                let b = self.ex(&Ty::coll(k2, et.clone()), d);
                let t = if op == "-" { "({} - {})".to_string() } else { format!("{{}}->{op}({{}})") };
                E::tpl(&t, vec![a, b], Ty::coll(rk, et))
            }
            5 | 6 if k.ordered() => {
                let c = self.ex(&ty, d);
                let x = self.ex(e, d);
                match self.rng.below(3) {
                    0 => E::tpl("{}->append({})", vec![c, x], ty),
                    1 => E::tpl("{}->prepend({})", vec![c, x], ty),
                    _ => {
                        let i = self.small_int(d);
                        E::tpl("{}->insertAt({}, {})", vec![c, i, x], ty)
                    }
                }
            }
            7 if k.ordered() => {
                let c = self.ex(&ty, d);
                let i = self.small_int(d);
                let j = self.small_int(d);
                let op = if k == Ck::Sequence { "subSequence" } else { "subOrderedSet" };
                E::tpl(&format!("{{}}->{op}({{}}, {{}})"), vec![c, i, j], ty)
            }
            8 if k.ordered() => {
                let c = self.ex(&ty, d);
                E::tpl("{}->reverse()", vec![c], ty)
            }
            9..=11 => {
                let c = self.ex(&ty, d);
                let x = self.fresh("x");
                let body = self.with_var(&x.clone(), et, |g| g.ex(&Ty::Bool, d));
                let op = *self.rng.pick(&["select", "reject"]);
                E::tpl(&format!("{{}}->{op}({x} | {{}})"), vec![c, body], ty).binding(&[&x])
            }
            12..=14 if matches!(k, Ck::Bag | Ck::Sequence) => {
                // collect: Set/Bag -> Bag, Sequence/OrderedSet -> Sequence
                let src_k = if k == Ck::Bag {
                    *self.rng.pick(&[Ck::Set, Ck::Bag])
                } else {
                    *self.rng.pick(&[Ck::Sequence, Ck::OrderedSet])
                };
                let st = self.elem_type(0);
                let c = self.ex(&Ty::coll(src_k, st.clone()), d);
                let x = self.fresh("x");
                let body = self.with_var(&x.clone(), st, |g| g.ex(e, d));
                if self.rng.pct(15) {
                    E::tpl(&format!("{{}}->xcollect({x} | {{}})"), vec![c, body], Ty::coll(Ck::Collection, et))
                        .binding(&[&x])
                } else {
                    E::tpl(&format!("{{}}->collect({x} | {{}})"), vec![c, body], ty).binding(&[&x])
                }
            }
            15 | 16 => {
                // sortedBy: Sequence/Bag -> Sequence, Set/OrderedSet -> OrderedSet
                let src_k = match k {
                    Ck::Sequence => *self.rng.pick(&[Ck::Sequence, Ck::Bag]),
                    Ck::OrderedSet => *self.rng.pick(&[Ck::Set, Ck::OrderedSet]),
                    _ => return self.leaf(&ty),
                };
                let c = self.ex(&Ty::coll(src_k, et.clone()), d);
                let x = self.fresh("x");
                let kt = self.rng.pick(&[Ty::Int, Ty::Str, Ty::Real]).clone();
                let body = self.with_var(&x.clone(), et, |g| g.ex(&kt, d));
                E::tpl(&format!("{{}}->sortedBy({x} | {{}})"), vec![c, body], ty).binding(&[&x])
            }
            17 if *e == Ty::Str && k == Ck::Sequence => {
                let s = self.ex(&Ty::Str, d);
                match self.rng.below(3) {
                    0 => E::tpl("{}.characters()", vec![s], ty),
                    1 => E::tpl("{}.tokenize()", vec![s], ty),
                    _ => {
                        let delims = quote(self.rng.pick(&[",", " ", ", ", ".", ""]));
                        E::tpl(&format!("{{}}.tokenize({delims})"), vec![s], ty)
                    }
                }
            }
            18 if *e == Ty::Obj(Cls::Node) => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                match k {
                    Ck::OrderedSet => E::tpl(self.rng.pick(&["{}.children", "{}.refs"]), vec![o], ty),
                    Ck::Set => {
                        if self.rng.pct(50) {
                            E::leaf("m.objectsOfType(FZ::Node)", ty)
                        } else {
                            E::tpl("{}->closure(x | x.children)", vec![o.retype(Ty::Obj(Cls::Node))], Ty::coll(Ck::Collection, et))
                        }
                    }
                    _ => {
                        let c = self.ex(&Ty::coll(Ck::OrderedSet, Ty::Obj(Cls::Node)), d);
                        E::tpl("{}->collect(x | x.refs)", vec![c], Ty::coll(Ck::Sequence, et))
                    }
                }
            }
            19 if *e == Ty::Str && k == Ck::OrderedSet => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.tags", vec![o], ty)
            }
            19 if *e == Ty::Int && k == Ck::Sequence => {
                let o = self.ex(&Ty::Obj(Cls::Node), d);
                E::tpl("{}.nums", vec![o], ty)
            }
            20 if e.is_coll() => {
                self.leaf(&ty)
            }
            20 => {
                // flatten of a nested collection
                let inner_k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, Ty::coll(inner_k, et.clone())), d);
                // OrderedSet::flatten is a Set
                let rk = if k == Ck::OrderedSet { Ck::Collection } else { k };
                E::tpl("{}->flatten()", vec![c], Ty::coll(rk, et))
            }
            21 => {
                let c = self.ex(&ty, d);
                let tp = (*e).clone();
                E::tpl(&format!("{{}}->selectByKind({})", tp.src()), vec![c], Ty::coll(Ck::Collection, et))
            }
            22 => {
                // the [cond] shorthand
                let c = self.ex(&ty, d);
                let x = self.fresh("x");
                let body = self.with_var(&x.clone(), et, |g| g.ex(&Ty::Bool, d));
                E::tpl(&format!("{{}}->xselect({x} | {{}})"), vec![c, body], Ty::coll(Ck::Collection, (*e).clone()))
                    .binding(&[&x])
            }
            23 if k == Ck::Sequence => {
                let l = self.ex(&Ty::List(Box::new(et.clone())), d);
                E::tpl("{}->asSequence()", vec![l], ty)
            }
            24 if k.unique() && *e == Ty::Str => {
                let dct = self.ex(&Ty::Dict(Box::new(Ty::Str), Box::new(Ty::Int)), d);
                E::tpl(&format!("{{}}->keys()->as{}()", k.name()), vec![dct], ty)
            }
            _ => self.leaf(&ty),
        }
    }

    fn list(&mut self, e: &Ty, d: u32) -> E {
        let ty = Ty::List(Box::new(e.clone()));
        match self.rng.below(6) {
            0 => {
                let k = *self.rng.pick(&Ck::CONCRETE);
                let c = self.ex(&Ty::coll(k, e.clone()), d);
                E::tpl("{}->asList()", vec![c], ty)
            }
            1 => {
                let l = self.ex(&ty, d);
                E::tpl("{}->reverse()", vec![l], ty)
            }
            2 => {
                let l = self.ex(&ty, d);
                E::tpl("{}->clone()", vec![l], ty)
            }
            3 if *e == Ty::Int => {
                let dct = self.ex(&Ty::Dict(Box::new(Ty::Str), Box::new(Ty::Int)), d);
                E::tpl("{}->values()", vec![dct], ty)
            }
            _ => self.leaf(&ty),
        }
    }

    fn dict(&mut self, k: &Ty, v: &Ty, _d: u32) -> E {
        self.leaf(&Ty::Dict(Box::new(k.clone()), Box::new(v.clone())))
    }
}

impl E {
    fn retype(mut self, t: Ty) -> E {
        self.ty = t;
        self
    }
}

/// One generated statement of an expression campaign: the statement text (QVTo) with the
/// expression at `{}` and the label it logs.
#[derive(Clone, Debug)]
pub struct Stmt {
    pub label: String,
    pub template: String,
    pub expr: E,
}

impl Stmt {
    /// The statement as QVTo: the template with the expression at `{}`, `$LABEL` the quoted
    /// label and `$VAR` a variable named after it.
    pub fn render(&self) -> String {
        let v = format!("v_{}", self.label.trim_matches('@'));
        self.template
            .replace("$LABEL", &quote(&self.label))
            .replace("$VAR", &v)
            .replace("{}", &self.expr.render())
    }
}

/// A statement that logs an expression under `label`: `log(label, e)`, an assignment to a
/// declared variable (QVTo's assignment conversions) and logging it, a conditional log, a
/// mutation of `l0`/`d0`, or an assertion.
pub fn statement(g: &mut Gen, label: &str, depth: u32) -> Stmt {
    let q = "$LABEL";
    match g.rng.below(20) {
        0..=3 => {
            let t = g.any_type(1);
            let e = g.ex(&t, depth);
            let v = "$VAR";
            Stmt {
                label: label.into(),
                template: format!("var {v} : {} := {{}}; log({q}, {v});", t.src()),
                expr: e,
            }
        }
        4 => {
            let e = g.ex(&Ty::Int, depth);
            Stmt {
                label: label.into(),
                template: format!("l0->add({{}}); log({q}, l0);"),
                expr: e,
            }
        }
        5 => {
            let e = g.ex(&Ty::Int, depth);
            let k = quote(g.rng.pick(&["a", "c", "", "b"]));
            Stmt {
                label: label.into(),
                template: format!("d0->put({k}, {{}}); log({q}, d0);"),
                expr: e,
            }
        }
        6 => {
            let e = g.ex(&Ty::Bool, depth);
            Stmt {
                label: label.into(),
                template: format!("log({q}) when {{}};"),
                expr: e,
            }
        }
        7 => {
            let e = g.ex(&Ty::Bool, depth);
            Stmt {
                label: label.into(),
                template: format!("assert warning ({{}}) with log({q});"),
                expr: e,
            }
        }
        8 => {
            // a collection variable of a declared kind, `+=` of an element
            let t = g.any_type(0);
            let k = *g.rng.pick(&Ck::CONCRETE);
            let e = g.ex(&Ty::coll(k, t.clone()), depth);
            let x = g.ex(&t, 1).render();
            let v = "$VAR";
            Stmt {
                label: label.into(),
                template: format!("var {v} : {}({}) := {{}}; {v} += {x}; log({q}, {v});", k.name(), t.src()),
                expr: e,
            }
        }
        _ => {
            let t = g.any_type(1);
            let e = g.ex(&t, depth);
            Stmt {
                label: label.into(),
                template: format!("log({q}, {{}});"),
                expr: e,
            }
        }
    }
}

/// A transformation that runs the statements in `main` on an input model `m`.
pub fn transformation(name: &str, stmts: &[Stmt]) -> String {
    let mut o = String::new();
    o.push_str(&format!(
        "modeltype FZ uses fz('{}');\n\ntransformation {name}(in m : FZ);\n\nmain() {{\n{}",
        super::FUZZ_NS_URI,
        BASE_DECLS
    ));
    for s in stmts {
        o.push('\t');
        o.push_str(&s.render());
        o.push('\n');
    }
    o.push_str("}\n");
    o
}
