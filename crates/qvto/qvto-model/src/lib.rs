//! Model API: traits over metamodels and model graphs (classes, features, containment,
//! opposites, ordering, uniqueness, proxies, resources, extents).
//!
//! The engine only talks to models through these traits, so it runs on `qvto-emf` and, inside
//! SimOxide, directly on `simoxide_model::raw::Graph`. See `DESIGN.md`, "Model API".
