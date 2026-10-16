//! Semantic analysis: module and import resolution, metamodel binding by nsURI, name resolution,
//! type checking, implicit conversions and shorthands. Produces the typed IR ([`ir`]) and
//! diagnostics with Eclipse's messages.

pub mod ir;
