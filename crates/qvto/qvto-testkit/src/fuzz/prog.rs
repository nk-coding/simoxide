//! Random small transformations over the fuzz metamodel.

use super::Rng;

/// A random transformation `name(in m : FZ, out o : FZ)`.
pub fn transformation(_rng: &mut Rng, name: &str) -> String {
    format!("modeltype FZ uses fz('{}');\ntransformation {name}(in m : FZ, out o : FZ);\nmain() {{}}\n", super::FUZZ_NS_URI)
}
