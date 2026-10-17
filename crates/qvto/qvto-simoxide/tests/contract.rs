//! `qvto-model`'s contract suite on the adapter (since M5 a view of the shared graph), with the
//! contract metamodel first in the registry (where SimOxide's packages normally are) and after
//! the session's metamodels. The PCM roles are covered by `tests/scripts.rs`.

use qvto_model::MetaRegistry;
use qvto_model::contract::build_metamodel;
use qvto_simoxide::{Session, SimStore};

fn leak<T>(x: T) -> &'static mut T {
    Box::leak(Box::new(x))
}

/// Objects of the contract metamodel in the `raw::Graph`.
mod graph_home {
    use super::*;
    qvto_model::contract_tests!(|| {
        let mut meta = MetaRegistry::new();
        let roles = build_metamodel(&mut meta);
        let session = leak(Session::with_registry(meta, &[]));
        assert!(session.is_graph_class(roles.node));
        let store = SimStore::new(leak(session.new_graph()), session);
        (store, roles)
    });
}

/// Objects of the contract metamodel in the session's `qvto_emf` graph (another metamodel).
mod other_home {
    use super::*;
    qvto_model::contract_tests!(|| {
        let session = leak(Session::with_registry(MetaRegistry::new(), &[]));
        let roles = build_metamodel(session.resource_set_mut().graph.meta_mut());
        assert!(!session.is_graph_class(roles.node));
        let store = SimStore::new(leak(session.new_graph()), session);
        (store, roles)
    });
}
