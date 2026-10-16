//! The model-store contract suite on `Graph`, with the metamodel built programmatically.

use qvto_emf::Graph;
use qvto_model::contract::build_metamodel;

qvto_model::contract_tests!(|| {
    let mut g = Graph::new();
    let roles = build_metamodel(g.meta_mut());
    (g, roles)
});
