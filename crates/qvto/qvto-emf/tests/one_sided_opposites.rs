//! References whose `eOpposite` is set on one side only (QVTo sets it so for
//! `opposites` declarations of intermediate class features). Expected values from EMF 2.41
//! (a dynamic `EPackage` built the same way, `qvto-emf/tests/data/OneSided.java`).

use qvto_emf::Graph;
use qvto_model::{Classifier, Datum, FeatureId, FeatureKind, ModelStore};

struct M {
    name: FeatureId,
    kids: FeatureId,
    parent: FeatureId,
    ops: FeatureId,
    owner: FeatureId,
    peer: FeatureId,
    peer2: FeatureId,
    node: qvto_model::ClassId,
}

fn metamodel(g: &mut Graph) -> M {
    let reg = g.meta_mut();
    let p = reg.add_package("p", Some("p"), Some("p"), None);
    let string = Classifier::DataType(reg.add_data_type(p, "S", Some("java.lang.String"), true));
    let node = reg.add_class(p, "Node", false, false);
    let r = |reg: &mut qvto_model::MetaRegistry, n: &str| {
        reg.add_feature(node, n, FeatureKind::Reference, Classifier::Class(node))
    };
    let name = reg.add_feature(node, "name", FeatureKind::Attribute, string);
    let kids = r(reg, "kids");
    let parent = r(reg, "parent");
    let ops = r(reg, "ops");
    let owner = r(reg, "owner");
    let peer = r(reg, "peer");
    let peer2 = r(reg, "peer2");
    for f in [kids, ops] {
        reg.feature_mut(f).containment = true;
        reg.feature_mut(f).upper = -1;
    }
    reg.feature_mut(kids).opposite = Some(parent); // the containment side only
    reg.feature_mut(owner).opposite = Some(ops); // the non-containment side only
    reg.feature_mut(peer).opposite = Some(peer2);
    reg.finish_classes().unwrap();
    M {
        name,
        kids,
        parent,
        ops,
        owner,
        peer,
        peer2,
        node,
    }
}

fn name(g: &Graph, m: &M, o: Option<qvto_emf::ObjId>) -> String {
    match o {
        None => "null".into(),
        Some(o) => match g.get(o, m.name) {
            Datum::Str(s) => s.to_string(),
            d => format!("{d:?}"),
        },
    }
}

#[test]
fn one_sided_opposites_as_emf() {
    let mut g = Graph::new();
    let m = metamodel(&mut g);
    let mk = |g: &mut Graph, n: &str| {
        let o = g.create(m.node).unwrap();
        g.set(o, m.name, Datum::Str(n)).unwrap();
        o
    };
    let (a, b, c) = (mk(&mut g, "a"), mk(&mut g, "b"), mk(&mut g, "c"));
    let single = |g: &Graph, o, f| g.get(o, f).as_obj();
    let container = |g: &Graph, o| g.container(o).map(|(p, _)| p);

    // 1: containment with a plain inverse: the inverse is set, no container
    g.add(a, m.kids, Datum::Obj(b), None).unwrap();
    assert_eq!(name(&g, &m, single(&g, b, m.parent)), "a");
    assert_eq!(container(&g, b), None);
    assert!(g.is_set(b, m.parent));
    // 2: setting the plain inverse does not move
    g.set(b, m.parent, Datum::Obj(c)).unwrap();
    assert_eq!(name(&g, &m, single(&g, b, m.parent)), "c");
    assert_eq!((g.len(a, m.kids), g.len(c, m.kids)), (1, 0));
    // 3: removal clears the inverse
    g.remove(a, m.kids, Datum::Obj(b)).unwrap();
    assert_eq!(single(&g, b, m.parent), None);
    assert_eq!(container(&g, b), None);
    // 4: a container reference with the opposite on its side only
    g.add(a, m.ops, Datum::Obj(b), None).unwrap();
    assert_eq!(name(&g, &m, single(&g, b, m.owner)), "a");
    assert_eq!(g.container(b), Some((a, m.ops)));
    // 5: setting it moves the object
    g.set(b, m.owner, Datum::Obj(c)).unwrap();
    assert_eq!(name(&g, &m, single(&g, b, m.owner)), "c");
    assert_eq!((g.len(a, m.ops), g.len(c, m.ops)), (0, 1));
    assert_eq!(container(&g, b), Some(c));
    // 6-8: plain references
    g.set(a, m.peer, Datum::Obj(b)).unwrap();
    assert_eq!(name(&g, &m, single(&g, b, m.peer2)), "a");
    assert_eq!(single(&g, a, m.peer2), None);
    g.set(b, m.peer2, Datum::Obj(c)).unwrap();
    assert_eq!(name(&g, &m, single(&g, a, m.peer)), "b");
    assert_eq!(single(&g, c, m.peer), None);
    g.set(a, m.peer, Datum::Obj(c)).unwrap();
    assert_eq!(single(&g, b, m.peer2), None);
    assert_eq!(name(&g, &m, single(&g, c, m.peer2)), "a");
}
