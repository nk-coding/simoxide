//! The model-store contract suite on `Graph`, with the metamodel built programmatically.

use qvto_emf::Graph;
use qvto_model::contract::build_metamodel;

qvto_model::contract_tests!(|| {
    let mut g = Graph::new();
    let roles = build_metamodel(g.meta_mut());
    (g, roles)
});

/// The same suite with the metamodel loaded from `contract.ecore` and registered.
mod from_ecore {
    use qvto_emf::ResourceSet;
    use qvto_model::contract::{CONTRACT_ECORE, Roles};

    qvto_model::contract_tests!(|| {
        let mut rs = ResourceSet::new();
        let r = rs.load_text("file:/contract/contract.ecore", CONTRACT_ECORE).unwrap();
        assert!(rs.graph.resource(r).errors.is_empty(), "{:?}", rs.graph.resource(r).errors);
        rs.register(r).unwrap();
        let roles = Roles::resolve(qvto_model::ModelStore::meta(&rs.graph)).unwrap();
        (rs.graph, roles)
    });
}
