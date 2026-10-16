//! The model-store contract test suite: EMF's list and reference semantics as executable checks.
//!
//! Every [`ModelStore`] implementation runs it with [`contract_tests!`](crate::contract_tests):
//!
//! ```ignore
//! qvto_model::contract_tests!(|| {
//!     let mut store = MyStore::new();
//!     let roles = qvto_model::contract::build_metamodel(store.meta_mut());
//!     (store, roles)
//! });
//! ```
//!
//! The tests use a small metamodel ([`CONTRACT_ECORE`], also built programmatically by
//! [`build_metamodel`]) whose features cover every kind of feature: single and many attributes
//! (unique and not), defaults, unsettable features, enums, containment with and without an
//! opposite, containment that resolves proxies (cross-resource containment), one-to-one,
//! one-to-many and many-to-many opposites, unique and non-unique reference lists. A store whose
//! metamodel is fixed (an adapter over generated tables) can fill [`Roles`] with features of its
//! own metamodel that have the same properties.
//!
//! The expected behaviour was checked against EMF 2.41 (dynamic objects): `qvto-emf` runs the
//! same scenarios as scripts on the Java side (`crates/qvto-emf/oracle`).

use crate::{
    ClassId, Classifier, Datum, EnumId, EnumLiteral, FeatureId, FeatureKind, MetaRegistry,
    ModelError, ModelStore,
};

/// The contract metamodel as an `.ecore` document (the same as [`build_metamodel`]).
pub const CONTRACT_ECORE: &str = include_str!("contract.ecore");
pub const CONTRACT_NS_URI: &str = "http://qvto-rs/contract";

/// Classes and features the contract tests use.
#[derive(Clone, Debug)]
pub struct Roles {
    pub base: ClassId,
    pub node: ClassId,
    pub sub: ClassId,
    pub kind: EnumId,
    pub name: FeatureId,
    pub id: FeatureId,
    pub count: FeatureId,
    pub label: FeatureId,
    pub opt: FeatureId,
    pub kind_attr: FeatureId,
    pub big: FeatureId,
    pub ratio: FeatureId,
    pub flag: FeatureId,
    pub boxed: FeatureId,
    pub tags: FeatureId,
    pub nums: FeatureId,
    pub children: FeatureId,
    pub parent: FeatureId,
    pub child: FeatureId,
    pub xchildren: FeatureId,
    pub links: FeatureId,
    pub bag: FeatureId,
    pub friend: FeatureId,
    pub owner: FeatureId,
    pub owned: FeatureId,
    pub targets: FeatureId,
    pub sources: FeatureId,
    pub only: FeatureId,
    pub frozen: FeatureId,
    pub extra: FeatureId,
}

impl Roles {
    /// Looks the roles up by name in a registry that has the contract package.
    pub fn resolve(reg: &MetaRegistry) -> Option<Roles> {
        let p = reg.package_by_ns_uri(CONTRACT_NS_URI)?;
        let base = reg.class_by_name(p, "Base")?;
        let node = reg.class_by_name(p, "Node")?;
        let sub = reg.class_by_name(p, "Sub")?;
        let kind = match reg.classifier_by_name(p, "Kind")? {
            Classifier::Enum(e) => e,
            _ => return None,
        };
        let f = |c: ClassId, n: &str| reg.feature_by_name(c, n);
        Some(Roles {
            base,
            node,
            sub,
            kind,
            name: f(node, "name")?,
            id: f(node, "id")?,
            count: f(node, "count")?,
            label: f(node, "label")?,
            opt: f(node, "opt")?,
            kind_attr: f(node, "kind")?,
            big: f(node, "big")?,
            ratio: f(node, "ratio")?,
            flag: f(node, "flag")?,
            boxed: f(node, "boxed")?,
            tags: f(node, "tags")?,
            nums: f(node, "nums")?,
            children: f(node, "children")?,
            parent: f(node, "parent")?,
            child: f(node, "child")?,
            xchildren: f(node, "xchildren")?,
            links: f(node, "links")?,
            bag: f(node, "bag")?,
            friend: f(node, "friend")?,
            owner: f(node, "owner")?,
            owned: f(node, "owned")?,
            targets: f(node, "targets")?,
            sources: f(node, "sources")?,
            only: f(node, "only")?,
            frozen: f(node, "frozen")?,
            extra: f(sub, "extra")?,
        })
    }
}

/// Adds the contract package to `reg` (same content as [`CONTRACT_ECORE`]).
pub fn build_metamodel(reg: &mut MetaRegistry) -> Roles {
    let p = reg.add_package("contract", Some(CONTRACT_NS_URI), Some("contract"), None);
    let string = Classifier::DataType(reg.add_data_type(p, "String", Some("java.lang.String"), true));
    let int = Classifier::DataType(reg.add_data_type(p, "Int", Some("int"), true));
    let integer = Classifier::DataType(reg.add_data_type(p, "Integer", Some("java.lang.Integer"), true));
    let long = Classifier::DataType(reg.add_data_type(p, "Long", Some("long"), true));
    let double = Classifier::DataType(reg.add_data_type(p, "Double", Some("double"), true));
    let boolean = Classifier::DataType(reg.add_data_type(p, "Boolean", Some("boolean"), true));
    let lit = |n: &str, v: i32| EnumLiteral {
        name: n.into(),
        value: v,
        literal: n.into(),
    };
    let kind = reg.add_enum(p, "Kind", vec![lit("A", 0), lit("B", 1), lit("C", 2)]);
    let base = reg.add_class(p, "Base", true, false);
    let node = reg.add_class(p, "Node", false, false);
    let sub = reg.add_class(p, "Sub", false, false);
    reg.class_mut(node).supers.push(base);
    reg.class_mut(sub).supers.push(node);
    let attr = |reg: &mut MetaRegistry, c, n: &str, t| reg.add_feature(c, n, FeatureKind::Attribute, t);
    let rf = |reg: &mut MetaRegistry, c, n: &str, t: ClassId| {
        reg.add_feature(c, n, FeatureKind::Reference, Classifier::Class(t))
    };
    let name = attr(reg, base, "name", string);
    let id = attr(reg, base, "id", string);
    reg.feature_mut(id).id = true;
    let count = attr(reg, node, "count", int);
    let label = attr(reg, node, "label", string);
    reg.feature_mut(label).default_literal = Some("x".into());
    let opt = attr(reg, node, "opt", string);
    reg.feature_mut(opt).unsettable = true;
    let kind_attr = attr(reg, node, "kind", Classifier::Enum(kind));
    let big = attr(reg, node, "big", long);
    let ratio = attr(reg, node, "ratio", double);
    let flag = attr(reg, node, "flag", boolean);
    let boxed = attr(reg, node, "boxed", integer);
    let tags = attr(reg, node, "tags", string);
    reg.feature_mut(tags).upper = -1;
    let nums = attr(reg, node, "nums", int);
    reg.feature_mut(nums).upper = -1;
    reg.feature_mut(nums).unique = false;
    let children = rf(reg, node, "children", node);
    let parent = rf(reg, node, "parent", node);
    let child = rf(reg, node, "child", node);
    let xchildren = rf(reg, node, "xchildren", node);
    let links = rf(reg, node, "links", node);
    let bag = rf(reg, node, "bag", node);
    let friend = rf(reg, node, "friend", node);
    let owner = rf(reg, node, "owner", node);
    let owned = rf(reg, node, "owned", node);
    let targets = rf(reg, node, "targets", node);
    let sources = rf(reg, node, "sources", node);
    let only = rf(reg, node, "only", sub);
    let frozen = attr(reg, node, "frozen", string);
    reg.feature_mut(frozen).changeable = false;
    for f in [children, xchildren, links, bag, owned, targets, sources] {
        reg.feature_mut(f).upper = -1;
    }
    for f in [children, parent, child] {
        reg.feature_mut(f).resolve_proxies = false;
    }
    reg.feature_mut(children).containment = true;
    reg.feature_mut(child).containment = true;
    reg.feature_mut(xchildren).containment = true;
    reg.feature_mut(parent).transient = true;
    reg.feature_mut(bag).unique = false;
    for (a, b) in [(children, parent), (owner, owned), (targets, sources)] {
        reg.feature_mut(a).opposite = Some(b);
        reg.feature_mut(b).opposite = Some(a);
    }
    reg.feature_mut(friend).opposite = Some(friend);
    let extra = attr(reg, sub, "extra", string);
    reg.finish_classes().expect("contract metamodel");
    reg.register_ns_uri(p);
    Roles {
        base,
        node,
        sub,
        kind,
        name,
        id,
        count,
        label,
        opt,
        kind_attr,
        big,
        ratio,
        flag,
        boxed,
        tags,
        nums,
        children,
        parent,
        child,
        xchildren,
        links,
        bag,
        friend,
        owner,
        owned,
        targets,
        sources,
        only,
        frozen,
        extra,
    }
}

/// Generates one `#[test]` per contract check. `$make` is an expression of a closure returning
/// `(store, Roles)` with a fresh store.
#[macro_export]
macro_rules! contract_tests {
    ($make:expr) => {
        $crate::contract_tests!(@each $make;
            create_defaults, is_set_single_data, type_checks, unique_data_list,
            non_unique_data_list, containment_many, containment_single, container_feature,
            opposite_one_to_one, opposite_one_to_many, opposite_many_to_many,
            reference_list_uniqueness, roots_and_resources, cross_resource_containment,
            delete, set_list, contents_order, unset_many, move_and_index_errors, not_changeable);
    };
    (@each $make:expr; $($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let (mut store, roles) = ($make)();
                $crate::contract::$name(&mut store, &roles);
            }
        )*
    };
}

// ---- helpers ------------------------------------------------------------------------------------

fn objs<S: ModelStore>(s: &S, o: S::Obj, f: FeatureId) -> Vec<S::Obj> {
    (0..s.len(o, f))
        .map(|i| match s.get_at(o, f, i) {
            Datum::Obj(x) => x,
            d => panic!("non-object {d:?} in reference list"),
        })
        .collect()
}

fn strs<S: ModelStore>(s: &S, o: S::Obj, f: FeatureId) -> Vec<String> {
    (0..s.len(o, f))
        .map(|i| match s.get_at(o, f, i) {
            Datum::Str(x) => x.to_string(),
            d => panic!("non-string {d:?}"),
        })
        .collect()
}

fn single<S: ModelStore>(s: &S, o: S::Obj, f: FeatureId) -> Option<S::Obj> {
    match s.get(o, f) {
        Datum::Obj(x) => Some(x),
        Datum::Null => None,
        d => panic!("non-object {d:?}"),
    }
}

fn mismatch<T>(e: Result<T, ModelError>) -> bool {
    matches!(e, Err(ModelError::TypeMismatch { .. }))
}

fn oob<T>(e: Result<T, ModelError>) -> bool {
    matches!(e, Err(ModelError::IndexOutOfBounds { .. }))
}

fn new<S: ModelStore>(s: &mut S, r: &Roles) -> S::Obj {
    s.create(r.node).expect("create Node")
}

// ---- the checks ---------------------------------------------------------------------------------

pub fn create_defaults<S: ModelStore>(s: &mut S, r: &Roles) {
    let n = new(s, r);
    assert_eq!(s.class_of(n), r.node);
    assert!(!s.is_proxy(n));
    assert_eq!(s.container(n), None);
    assert_eq!(s.resource_of(n), None);
    assert_eq!(s.direct_resource(n), None);
    assert_eq!(s.get(n, r.name), Datum::Null);
    assert_eq!(s.get(n, r.count), Datum::Int(0));
    assert_eq!(s.get(n, r.label), Datum::Str("x"));
    assert_eq!(s.get(n, r.opt), Datum::Null);
    assert_eq!(s.get(n, r.kind_attr), Datum::Enum(r.kind, 0));
    assert_eq!(s.get(n, r.big), Datum::Long(0));
    assert_eq!(s.get(n, r.ratio), Datum::Double(0.0));
    assert_eq!(s.get(n, r.flag), Datum::Bool(false));
    assert_eq!(s.get(n, r.boxed), Datum::Null);
    assert_eq!(s.get(n, r.child), Datum::Null);
    assert_eq!(s.get(n, r.parent), Datum::Null);
    assert_eq!(s.len(n, r.tags), 0);
    assert_eq!(s.len(n, r.children), 0);
    for f in [r.name, r.count, r.label, r.opt, r.kind_attr, r.tags, r.children, r.child, r.friend] {
        assert!(!s.is_set(n, f));
    }
    assert!(matches!(s.create(r.base), Err(ModelError::Abstract(_))));
    let x = s.create(r.sub).unwrap();
    assert_eq!(s.class_of(x), r.sub);
    assert_eq!(s.get(x, r.extra), Datum::Null);
}

pub fn is_set_single_data<S: ModelStore>(s: &mut S, r: &Roles) {
    let n = new(s, r);
    s.set(n, r.name, Datum::Str("a")).unwrap();
    assert!(s.is_set(n, r.name));
    assert_eq!(s.get(n, r.name), Datum::Str("a"));
    s.set(n, r.name, Datum::Null).unwrap();
    assert!(!s.is_set(n, r.name));
    // non-unsettable: "set" means "differs from the default"
    s.set(n, r.count, Datum::Int(0)).unwrap();
    assert!(!s.is_set(n, r.count));
    s.set(n, r.count, Datum::Int(5)).unwrap();
    assert!(s.is_set(n, r.count));
    s.unset(n, r.count).unwrap();
    assert!(!s.is_set(n, r.count));
    assert_eq!(s.get(n, r.count), Datum::Int(0));
    // null into a primitive feature resets it to the default
    s.set(n, r.count, Datum::Int(3)).unwrap();
    s.set(n, r.count, Datum::Null).unwrap();
    assert_eq!(s.get(n, r.count), Datum::Int(0));
    assert!(!s.is_set(n, r.count));
    // null into a feature with a non-null default literal is an explicit null
    s.set(n, r.label, Datum::Null).unwrap();
    assert!(s.is_set(n, r.label));
    assert_eq!(s.get(n, r.label), Datum::Null);
    s.set(n, r.label, Datum::Str("x")).unwrap();
    assert!(!s.is_set(n, r.label));
    // unsettable: set (even to null) until unset
    s.set(n, r.opt, Datum::Null).unwrap();
    assert!(s.is_set(n, r.opt));
    assert_eq!(s.get(n, r.opt), Datum::Null);
    s.unset(n, r.opt).unwrap();
    assert!(!s.is_set(n, r.opt));
    // enums default to the first literal
    s.set(n, r.kind_attr, Datum::Enum(r.kind, 0)).unwrap();
    assert!(!s.is_set(n, r.kind_attr));
    s.set(n, r.kind_attr, Datum::Enum(r.kind, 2)).unwrap();
    assert!(s.is_set(n, r.kind_attr));
    assert_eq!(s.get(n, r.kind_attr), Datum::Enum(r.kind, 2));
    s.set(n, r.big, Datum::Long(1 << 40)).unwrap();
    assert_eq!(s.get(n, r.big), Datum::Long(1 << 40));
    s.set(n, r.ratio, Datum::Double(-0.5)).unwrap();
    assert_eq!(s.get(n, r.ratio), Datum::Double(-0.5));
    s.set(n, r.boxed, Datum::Int(0)).unwrap();
    assert!(s.is_set(n, r.boxed));
    assert_eq!(s.get(n, r.boxed), Datum::Int(0));
}

pub fn type_checks<S: ModelStore>(s: &mut S, r: &Roles) {
    let n = new(s, r);
    let sub = s.create(r.sub).unwrap();
    assert!(mismatch(s.set(n, r.count, Datum::Str("1"))));
    assert!(mismatch(s.set(n, r.count, Datum::Long(1))));
    assert!(mismatch(s.set(n, r.name, Datum::Int(1))));
    assert!(mismatch(s.set(n, r.child, Datum::Str("x"))));
    assert!(mismatch(s.set(n, r.only, Datum::Obj(n))));
    s.set(n, r.only, Datum::Obj(sub)).unwrap();
    assert_eq!(single(s, n, r.only), Some(sub));
    assert!(mismatch(s.add(n, r.tags, Datum::Int(1), None)));
    assert!(matches!(
        s.add(n, r.links, Datum::Null, None),
        Err(ModelError::NullNotAllowed(_))
    ));
    assert!(matches!(
        s.set(n, r.extra, Datum::Str("e")),
        Err(ModelError::NoSuchFeature { .. })
    ));
    s.set(sub, r.extra, Datum::Str("e")).unwrap();
    assert_eq!(s.get(sub, r.extra), Datum::Str("e"));
}

pub fn unique_data_list<S: ModelStore>(s: &mut S, r: &Roles) {
    let n = new(s, r);
    assert_eq!(s.add(n, r.tags, Datum::Str("a"), None), Ok(true));
    assert_eq!(s.add(n, r.tags, Datum::Str("b"), None), Ok(true));
    assert_eq!(s.add(n, r.tags, Datum::Str("a"), None), Ok(false));
    assert!(s.is_set(n, r.tags));
    assert!(matches!(
        s.add(n, r.tags, Datum::Str("a"), Some(0)),
        Err(ModelError::Duplicate(_))
    ));
    s.add(n, r.tags, Datum::Str("c"), Some(1)).unwrap();
    assert_eq!(strs(s, n, r.tags), ["a", "c", "b"]);
    assert!(matches!(
        s.set_at(n, r.tags, 0, Datum::Str("b")),
        Err(ModelError::Duplicate(_))
    ));
    s.set_at(n, r.tags, 0, Datum::Str("a")).unwrap();
    s.set_at(n, r.tags, 0, Datum::Str("z")).unwrap();
    assert_eq!(strs(s, n, r.tags), ["z", "c", "b"]);
    assert_eq!(s.remove(n, r.tags, Datum::Str("c")), Ok(true));
    assert_eq!(s.remove(n, r.tags, Datum::Str("nope")), Ok(false));
    assert_eq!(strs(s, n, r.tags), ["z", "b"]);
    s.clear(n, r.tags).unwrap();
    assert_eq!(s.len(n, r.tags), 0);
    assert!(!s.is_set(n, r.tags));
}

pub fn non_unique_data_list<S: ModelStore>(s: &mut S, r: &Roles) {
    let n = new(s, r);
    for v in [1, 1, 2, 1] {
        assert_eq!(s.add(n, r.nums, Datum::Int(v), None), Ok(true));
    }
    s.add(n, r.nums, Datum::Int(2), Some(0)).unwrap();
    let get = |s: &S| (0..s.len(n, r.nums)).map(|i| s.get_at(n, r.nums, i).as_i64().unwrap()).collect::<Vec<_>>();
    assert_eq!(get(s), [2, 1, 1, 2, 1]);
    assert_eq!(s.remove(n, r.nums, Datum::Int(1)), Ok(true));
    assert_eq!(get(s), [2, 1, 2, 1]);
    s.set_at(n, r.nums, 0, Datum::Int(1)).unwrap();
    assert_eq!(get(s), [1, 1, 2, 1]);
}

pub fn containment_many<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p1, p2, c, d) = (new(s, r), new(s, r), new(s, r), new(s, r));
    assert_eq!(s.add(p1, r.children, Datum::Obj(c), None), Ok(true));
    assert_eq!(s.container(c), Some((p1, r.children)));
    assert_eq!(single(s, c, r.parent), Some(p1));
    assert_eq!(s.add(p1, r.children, Datum::Obj(c), None), Ok(false));
    assert_eq!(objs(s, p1, r.children), [c]);
    // moving to another container removes it from the first
    assert_eq!(s.add(p2, r.children, Datum::Obj(c), None), Ok(true));
    assert!(objs(s, p1, r.children).is_empty());
    assert_eq!(s.container(c), Some((p2, r.children)));
    s.add(p2, r.children, Datum::Obj(d), Some(0)).unwrap();
    assert_eq!(objs(s, p2, r.children), [d, c]);
    assert_eq!(s.remove(p2, r.children, Datum::Obj(d)), Ok(true));
    assert_eq!(s.container(d), None);
    assert_eq!(single(s, d, r.parent), None);
    s.clear(p2, r.children).unwrap();
    assert_eq!(s.container(c), None);
    // a child moved within the same list keeps its container
    s.add(p1, r.children, Datum::Obj(c), None).unwrap();
    s.add(p1, r.children, Datum::Obj(d), None).unwrap();
    s.move_to(p1, r.children, 0, 1).unwrap();
    assert_eq!(objs(s, p1, r.children), [d, c]);
    assert_eq!(s.container(c), Some((p1, r.children)));
    s.remove_at(p1, r.children, 0).unwrap();
    assert_eq!(s.container(d), None);
}

pub fn containment_single<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p, q, c, d) = (new(s, r), new(s, r), new(s, r), new(s, r));
    s.add(p, r.children, Datum::Obj(c), None).unwrap();
    s.set(p, r.child, Datum::Obj(c)).unwrap();
    assert!(objs(s, p, r.children).is_empty());
    assert_eq!(s.container(c), Some((p, r.child)));
    assert_eq!(single(s, c, r.parent), None);
    s.set(q, r.child, Datum::Obj(c)).unwrap();
    assert_eq!(single(s, p, r.child), None);
    assert!(!s.is_set(p, r.child));
    assert_eq!(s.container(c), Some((q, r.child)));
    s.set(q, r.child, Datum::Obj(d)).unwrap();
    assert_eq!(s.container(c), None);
    assert_eq!(s.container(d), Some((q, r.child)));
    s.unset(q, r.child).unwrap();
    assert_eq!(s.container(d), None);
    assert!(!s.is_set(q, r.child));
}

pub fn container_feature<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p1, p2, c, d) = (new(s, r), new(s, r), new(s, r), new(s, r));
    s.add(p2, r.children, Datum::Obj(d), None).unwrap();
    s.set(c, r.parent, Datum::Obj(p1)).unwrap();
    assert_eq!(objs(s, p1, r.children), [c]);
    assert!(s.is_set(c, r.parent));
    s.set(c, r.parent, Datum::Obj(p2)).unwrap();
    assert!(objs(s, p1, r.children).is_empty());
    assert_eq!(objs(s, p2, r.children), [d, c]);
    s.set(c, r.parent, Datum::Null).unwrap();
    assert_eq!(s.container(c), None);
    assert_eq!(objs(s, p2, r.children), [d]);
}

pub fn opposite_one_to_one<S: ModelStore>(s: &mut S, r: &Roles) {
    let (a, b, c) = (new(s, r), new(s, r), new(s, r));
    s.set(a, r.friend, Datum::Obj(b)).unwrap();
    assert_eq!(single(s, b, r.friend), Some(a));
    s.set(a, r.friend, Datum::Obj(c)).unwrap();
    assert_eq!(single(s, b, r.friend), None);
    assert_eq!(single(s, c, r.friend), Some(a));
    s.set(b, r.friend, Datum::Obj(c)).unwrap();
    assert_eq!(single(s, c, r.friend), Some(b));
    assert_eq!(single(s, a, r.friend), None);
    s.unset(b, r.friend).unwrap();
    assert_eq!(single(s, c, r.friend), None);
    assert_eq!(single(s, b, r.friend), None);
}

pub fn opposite_one_to_many<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p, q, x, y, z) = (new(s, r), new(s, r), new(s, r), new(s, r), new(s, r));
    s.set(x, r.owner, Datum::Obj(p)).unwrap();
    assert_eq!(objs(s, p, r.owned), [x]);
    s.add(p, r.owned, Datum::Obj(y), None).unwrap();
    assert_eq!(single(s, y, r.owner), Some(p));
    assert_eq!(objs(s, p, r.owned), [x, y]);
    s.set(x, r.owner, Datum::Obj(q)).unwrap();
    assert_eq!(objs(s, p, r.owned), [y]);
    assert_eq!(objs(s, q, r.owned), [x]);
    assert_eq!(s.remove(p, r.owned, Datum::Obj(y)), Ok(true));
    assert_eq!(single(s, y, r.owner), None);
    s.add(q, r.owned, Datum::Obj(z), Some(0)).unwrap();
    assert_eq!(objs(s, q, r.owned), [z, x]);
    s.add(p, r.owned, Datum::Obj(z), None).unwrap();
    assert_eq!(objs(s, q, r.owned), [x]);
    assert_eq!(single(s, z, r.owner), Some(p));
    s.clear(q, r.owned).unwrap();
    assert_eq!(single(s, x, r.owner), None);
}

pub fn opposite_many_to_many<S: ModelStore>(s: &mut S, r: &Roles) {
    let (a, b, c, d) = (new(s, r), new(s, r), new(s, r), new(s, r));
    s.add(a, r.targets, Datum::Obj(b), None).unwrap();
    assert_eq!(objs(s, b, r.sources), [a]);
    s.add(b, r.sources, Datum::Obj(c), None).unwrap();
    assert_eq!(objs(s, c, r.targets), [b]);
    assert_eq!(objs(s, b, r.sources), [a, c]);
    s.add(a, r.targets, Datum::Obj(c), None).unwrap();
    s.move_to(a, r.targets, 0, 1).unwrap();
    assert_eq!(objs(s, a, r.targets), [c, b]);
    assert_eq!(objs(s, c, r.sources), [a]);
    s.set_at(a, r.targets, 0, Datum::Obj(d)).unwrap();
    assert_eq!(objs(s, a, r.targets), [d, b]);
    assert!(objs(s, c, r.sources).is_empty());
    assert_eq!(objs(s, d, r.sources), [a]);
    assert_eq!(s.remove(a, r.targets, Datum::Obj(b)), Ok(true));
    assert_eq!(objs(s, b, r.sources), [c]);
}

pub fn reference_list_uniqueness<S: ModelStore>(s: &mut S, r: &Roles) {
    let (a, b, c) = (new(s, r), new(s, r), new(s, r));
    assert_eq!(s.add(a, r.links, Datum::Obj(b), None), Ok(true));
    assert_eq!(s.add(a, r.links, Datum::Obj(b), None), Ok(false));
    assert!(matches!(
        s.add(a, r.links, Datum::Obj(b), Some(0)),
        Err(ModelError::Duplicate(_))
    ));
    assert_eq!(s.add(a, r.bag, Datum::Obj(b), None), Ok(true));
    assert_eq!(s.add(a, r.bag, Datum::Obj(b), None), Ok(true));
    s.add(a, r.bag, Datum::Obj(c), Some(1)).unwrap();
    assert_eq!(objs(s, a, r.bag), [b, c, b]);
    assert_eq!(s.remove(a, r.bag, Datum::Obj(b)), Ok(true));
    assert_eq!(objs(s, a, r.bag), [c, b]);
}

pub fn roots_and_resources<S: ModelStore>(s: &mut S, r: &Roles) {
    let (a, c) = (new(s, r), new(s, r));
    let r1 = s.create_resource("mem:/r1");
    assert_eq!(s.resource_uri(r1), "mem:/r1");
    assert_eq!(s.add_root(r1, a, None), Ok(true));
    assert_eq!(s.add_root(r1, a, None), Ok(false));
    assert_eq!(s.roots(r1), [a]);
    assert_eq!(s.resource_of(a), Some(r1));
    assert_eq!(s.direct_resource(a), Some(r1));
    s.add(a, r.children, Datum::Obj(c), None).unwrap();
    assert_eq!(s.resource_of(c), Some(r1));
    assert_eq!(s.direct_resource(c), None);
    // a contained object that becomes a root leaves its container
    assert_eq!(s.add_root(r1, c, None), Ok(true));
    assert!(objs(s, a, r.children).is_empty());
    assert_eq!(s.container(c), None);
    assert_eq!(s.roots(r1), [a, c]);
    // a root that becomes contained leaves the root list
    s.add(a, r.children, Datum::Obj(c), None).unwrap();
    assert_eq!(s.roots(r1), [a]);
    assert_eq!(s.direct_resource(c), None);
    // moving a root to another resource
    let r2 = s.create_resource("mem:/r2");
    s.add_root(r2, a, None).unwrap();
    assert!(s.roots(r1).is_empty());
    assert_eq!(s.roots(r2), [a]);
    assert_eq!(s.resource_of(c), Some(r2));
    assert!(s.remove_root(r2, a));
    assert!(!s.remove_root(r2, a));
    assert_eq!(s.resource_of(a), None);
    assert_eq!(s.resource_of(c), None);
}

pub fn cross_resource_containment<S: ModelStore>(s: &mut S, r: &Roles) {
    let (a, d, e) = (new(s, r), new(s, r), new(s, r));
    let r1 = s.create_resource("mem:/r1");
    let r2 = s.create_resource("mem:/r2");
    s.add_root(r1, a, None).unwrap();
    s.add_root(r2, d, None).unwrap();
    // `xchildren` resolves proxies: the child stays a root of its own resource
    s.add(a, r.xchildren, Datum::Obj(d), None).unwrap();
    assert_eq!(s.container(d), Some((a, r.xchildren)));
    assert_eq!(s.roots(r2), [d]);
    assert_eq!(s.direct_resource(d), Some(r2));
    assert_eq!(s.resource_of(d), Some(r2));
    // a contained object added as a root keeps its container in that case
    s.add(a, r.xchildren, Datum::Obj(e), None).unwrap();
    s.add_root(r2, e, None).unwrap();
    assert_eq!(s.container(e), Some((a, r.xchildren)));
    assert_eq!(s.roots(r2), [d, e]);
    assert_eq!(objs(s, a, r.xchildren), [d, e]);
}

pub fn delete<S: ModelStore>(s: &mut S, r: &Roles) {
    let (a, b, c, d, e) = (new(s, r), new(s, r), new(s, r), new(s, r), new(s, r));
    let res = s.create_resource("mem:/r");
    s.add_root(res, a, None).unwrap();
    s.add_root(res, b, None).unwrap();
    s.add(a, r.children, Datum::Obj(c), None).unwrap();
    s.add(a, r.children, Datum::Obj(d), None).unwrap();
    s.add(b, r.children, Datum::Obj(e), None).unwrap();
    s.add(b, r.links, Datum::Obj(c), None).unwrap();
    s.add(b, r.links, Datum::Obj(d), None).unwrap();
    s.set(e, r.friend, Datum::Obj(c)).unwrap();
    s.add(e, r.targets, Datum::Obj(c), None).unwrap();
    s.add(c, r.links, Datum::Obj(d), None).unwrap();
    s.delete(c);
    assert_eq!(objs(s, a, r.children), [d]);
    assert_eq!(s.container(c), None);
    assert_eq!(objs(s, b, r.links), [d]);
    assert_eq!(single(s, e, r.friend), None);
    assert!(objs(s, e, r.targets).is_empty());
    // the deleted object's own references stay (EcoreUtil.delete only removes usages)
    assert_eq!(objs(s, c, r.links), [d]);
    s.delete(b);
    assert_eq!(s.roots(res), [a]);
}

pub fn set_list<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p, a, b, c, d) = (new(s, r), new(s, r), new(s, r), new(s, r), new(s, r));
    for x in [a, b, c] {
        s.add(p, r.children, Datum::Obj(x), None).unwrap();
    }
    s.set_list(p, r.children, &[Datum::Obj(c), Datum::Obj(a), Datum::Obj(d)]).unwrap();
    assert_eq!(objs(s, p, r.children), [c, a, d]);
    assert_eq!(s.container(b), None);
    assert_eq!(s.container(d), Some((p, r.children)));
    for t in ["x", "y", "z"] {
        s.add(p, r.tags, Datum::Str(t), None).unwrap();
    }
    s.set_list(p, r.tags, &[Datum::Str("z"), Datum::Str("y")]).unwrap();
    assert_eq!(strs(s, p, r.tags), ["z", "y"]);
    s.set_list(p, r.owned, &[Datum::Obj(a), Datum::Obj(b)]).unwrap();
    assert_eq!(single(s, b, r.owner), Some(p));
    s.set_list(p, r.owned, &[Datum::Obj(b)]).unwrap();
    assert_eq!(single(s, a, r.owner), None);
    assert_eq!(objs(s, p, r.owned), [b]);
}

pub fn contents_order<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p, a, b, c, d) = (new(s, r), new(s, r), new(s, r), new(s, r), new(s, r));
    s.add(p, r.xchildren, Datum::Obj(d), None).unwrap();
    s.set(p, r.child, Datum::Obj(c)).unwrap();
    s.add(p, r.children, Datum::Obj(a), None).unwrap();
    s.add(a, r.children, Datum::Obj(b), None).unwrap();
    assert_eq!(s.contents(p), [a, c, d]);
    assert_eq!(s.all_contents(p), [a, b, c, d]);
}

pub fn unset_many<S: ModelStore>(s: &mut S, r: &Roles) {
    let (p, a) = (new(s, r), new(s, r));
    s.add(p, r.children, Datum::Obj(a), None).unwrap();
    s.add(p, r.tags, Datum::Str("t"), None).unwrap();
    s.unset(p, r.children).unwrap();
    s.unset(p, r.tags).unwrap();
    assert_eq!(s.len(p, r.children), 0);
    assert_eq!(s.container(a), None);
    assert!(!s.is_set(p, r.tags));
}

pub fn move_and_index_errors<S: ModelStore>(s: &mut S, r: &Roles) {
    let p = new(s, r);
    s.add(p, r.tags, Datum::Str("a"), None).unwrap();
    s.add(p, r.tags, Datum::Str("b"), None).unwrap();
    assert!(oob(s.move_to(p, r.tags, 2, 0)));
    assert!(oob(s.move_to(p, r.tags, 0, 2)));
    assert!(oob(s.remove_at(p, r.tags, 2)));
    assert!(oob(s.set_at(p, r.tags, 2, Datum::Str("c"))));
    assert!(oob(s.add(p, r.tags, Datum::Str("c"), Some(3))));
    s.add(p, r.tags, Datum::Str("c"), Some(2)).unwrap();
    s.move_to(p, r.tags, 0, 2).unwrap();
    assert_eq!(strs(s, p, r.tags), ["c", "a", "b"]);
}

pub fn not_changeable<S: ModelStore>(s: &mut S, r: &Roles) {
    let n = new(s, r);
    assert!(matches!(s.set(n, r.frozen, Datum::Str("a")), Err(ModelError::NotChangeable(_))));
    assert!(matches!(s.unset(n, r.frozen), Err(ModelError::NotChangeable(_))));
    assert_eq!(s.get(n, r.frozen), Datum::Null);
}
