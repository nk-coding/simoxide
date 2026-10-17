//! The graph's change journal: effective changes with their side effects, in EMF's order (the
//! recording points of the former `qvto-simoxide` adapter), and nothing while recording is off.

use qvto_model::contract::build_metamodel;
use qvto_model::{Datum, ModelStore, OwnedDatum};
use simoxide_emf::{ChangeKind as K, Graph, IdPolicy};

fn kinds(g: &mut Graph) -> Vec<(u32, K)> {
    g.take_journal()
        .changes
        .into_iter()
        .map(|c| (c.obj.0, c.kind))
        .collect()
}

#[test]
fn records_effective_changes_and_side_effects() {
    let mut g = Graph::new();
    let r = build_metamodel(g.meta_mut());
    let res = g.create_resource("file:/m.xmi");
    let a = g.create(r.node).unwrap();
    let b = g.create(r.node).unwrap();
    let c = g.create(r.node).unwrap();
    g.add_root(res, a, None).unwrap();
    assert!(g.journal().is_none(), "nothing recorded while off");

    g.start_journal();
    // containment with an opposite: the list change, then the child's container
    g.add(a, r.children, Datum::Obj(b), None).unwrap();
    // one-to-many opposite: owner set on c, c added to a.owned
    g.set(c, r.owner, Datum::Obj(a)).unwrap();
    // attribute set and unset
    g.set(b, r.name, Datum::Str("x")).unwrap();
    g.unset(b, r.name).unwrap();
    // move b from a.children to c.children: removed from a, container changes
    g.add(c, r.children, Datum::Obj(b), None).unwrap();
    let j = kinds(&mut g);
    let f = |k: &K| match k {
        K::Added { feature, .. }
        | K::Removed { feature, .. }
        | K::Set { feature, .. }
        | K::Unset { feature, .. } => Some(*feature),
        _ => None,
    };
    let expect = vec![
        (a.0, Some(r.children)),
        (b.0, None), // container a.children
        (a.0, Some(r.owned)),
        (c.0, Some(r.owner)),
        (b.0, Some(r.name)),
        (b.0, Some(r.name)),
        (c.0, Some(r.children)),
        (a.0, Some(r.children)), // removed from the old container
        (b.0, None),             // container c.children
    ];
    let got: Vec<(u32, Option<qvto_model::FeatureId>)> =
        j.iter().map(|(o, k)| (*o, f(k))).collect();
    assert_eq!(got, expect, "{j:#?}");
    assert!(matches!(
        &j[1].1,
        K::Container { old: None, new: Some((p, _)) } if *p == a
    ));
    assert!(matches!(&j[5].1, K::Unset { .. }));
    assert!(matches!(
        &j[7].1,
        K::Removed { index: 0, value: OwnedDatum::Obj(x), .. } if *x == b
    ));
}

#[test]
fn roots_delete_and_ids() {
    let mut g = Graph::new();
    let r = build_metamodel(g.meta_mut());
    let res = g.create_resource("file:/m.xmi");
    let a = g.create(r.node).unwrap();
    g.add_root(res, a, None).unwrap();
    let b = g.create(r.node).unwrap();
    g.add(a, r.children, Datum::Obj(b), None).unwrap();
    g.set(a, r.friend, Datum::Obj(b)).unwrap();
    g.start_journal();
    // a contained object becomes a root: out of its container first
    g.add_root(res, b, None).unwrap();
    g.delete(b);
    let j = kinds(&mut g);
    assert!(matches!(j[0].1, K::RootAdded { index: 1, .. }));
    assert!(matches!(j[1].1, K::Removed { .. }));
    assert!(matches!(j[2].1, K::Container { new: None, .. }));
    assert!(matches!(j.last().unwrap().1, K::Deleted));
    assert!(j.iter().any(|(_, k)| matches!(k, K::RootRemoved { .. })));

    // the id policy: new objects of the id class get a reproducible id
    g.set_id_target(r.node, r.name);
    g.set_id_policy(IdPolicy::Uuid { seed: 7 });
    let x = g.create(r.node).unwrap();
    let id = g.get(x, r.name).as_str().unwrap().to_string();
    assert_eq!(id.len(), 23);
    let mut h = g.clone();
    let y1 = g.create(r.node).unwrap();
    let y2 = h.create(r.node).unwrap();
    assert_eq!(
        g.get(y1, r.name),
        h.get(y2, r.name),
        "a copy draws the same ids"
    );
    let j = kinds(&mut g);
    assert!(matches!(j[0].1, K::Created(c) if c == r.node));
    assert!(matches!(&j[1].1, K::Set { feature, .. } if *feature == r.name));
}

#[test]
fn graph_is_plain_data() {
    fn value<T: Clone + Send + Sync>() {}
    value::<Graph>();
}
