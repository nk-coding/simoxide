//! `qvto-model`'s contract suite on the adapter, in both homes of its objects.
//!
//! The suite's metamodel is not one of SimOxide's, but the adapter's graph code is driven by the
//! registry alone, so the contract metamodel can be put where SimOxide's packages normally are
//! (first in the registry, classes stored in a `raw::Graph`). The PCM roles are covered by
//! `tests/scripts.rs`, which runs random operations on PCM classes against `qvto_emf`.

use qvto_model::MetaRegistry;
use qvto_model::contract::build_metamodel;
use qvto_simoxide::{Session, SimStore};
use simoxide_model::raw;

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
        let store = SimStore::new(leak(raw::Graph::default()), session);
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
        let store = SimStore::new(leak(raw::Graph::default()), session);
        (store, roles)
    });
}
