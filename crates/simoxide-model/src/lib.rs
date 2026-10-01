//! Loader for Palladio Component Model (PCM 5.2, as used by SimuLizar 5.2.2) XMI files.
//!
//! Two layers:
//! - [`raw`]: a generic object graph reproducing EMF's loading semantics (defaults, IDREF and
//!   proxy resolution, opposites, ID lookup order) for every metaclass, driven by metamodel tables
//!   generated from the 5.2.2 `.ecore` files ([`meta`]). Its canonical dump ([`canon`]) is
//!   compared with a dump made by real EMF (`reference/oracles/pcm-model`).
//! - [`model`]: the typed, arena/index-based representation of the simulation subset, built
//!   from the graph ([`Model`]), plus a validation pass ([`validate`]).
//!
//! The metamodel tables (`src/meta/generated.rs`) and the bundled default models (`models/`:
//! `Palladio.resourcetype`, `PrimitiveTypes.repository`, `commonMetrics.metricspec`, ...) are
//! extracted from the product jars by `tools/gen_meta.py`. StoEx strings are kept raw.
//!
//! Models can also be loaded from memory ([`load_memory`], [`Loader::add_file`]), e.g. when
//! they arrive over the network. A [`ParseCache`] shared by loaders avoids parsing files again
//! that many models have in common; the bundled default models are parsed once per process.
//!
//! ```no_run
//! let m = simoxide_model::load_dir("corpus/minimal").unwrap();
//! for d in m.diagnostics.iter() {
//!     eprintln!("{d}");
//! }
//! for s in m.usage_scenarios.iter() {
//!     println!("{} {:?}", s.name, s.workload);
//! }
//! ```

mod builder;
pub mod canon;
pub mod diag;
mod fxhash;
pub mod load;
pub mod meta;
pub mod model;
pub mod raw;
pub mod validate;
mod xmi;

pub use diag::{Diagnostic, Diagnostics, Level};
pub use load::{LoadOptions, Loader, ParseCache};
pub use model::*;

impl Loader {
    /// Resolves all references, builds the typed model and validates it.
    pub fn build(self) -> Model {
        let (g, d) = self.finish();
        build_from_graph(g, d)
    }
}

/// Builds and validates the typed model from a resolved graph (the second half of
/// [`Loader::build`], after [`Loader::finish`]).
pub fn build_from_graph(g: raw::Graph, d: Diagnostics) -> Model {
    let mut m = builder::build(g, d);
    validate::validate(&mut m);
    m
}

/// Loads all model files of a directory (see [`load::MODEL_EXTENSIONS`]) with default options.
pub fn load_dir(dir: impl AsRef<std::path::Path>) -> std::io::Result<Model> {
    let mut l = Loader::default();
    l.load_dir(dir)?;
    Ok(l.build())
}

/// Loads models from memory, without touching the file system: `files` are (file name, XMI
/// content) pairs, e.g. received over the network. Relative names are placed in
/// [`load::MEMORY_DIR`], so `href`s between the files resolve as in a directory. Like
/// [`load_dir`], all model files (see [`load::MODEL_EXTENSIONS`]) are loaded, sorted by name,
/// plus what they reference; other files are only loaded when referenced. The result equals
/// [`load_dir`] of a directory with the same files (up to the resource URIs and paths).
pub fn load_memory<N, C>(files: impl IntoIterator<Item = (N, C)>) -> Model
where
    N: AsRef<std::path::Path>,
    C: Into<Vec<u8>>,
{
    let mut l = memory_loader(files);
    l.load_memory_dir(load::MEMORY_DIR);
    l.build()
}

/// Like [`load_memory`], but loads only the given entry files (names as in `files`) and what
/// they reference, like [`load_files`].
pub fn load_memory_entries<N, C, E>(files: impl IntoIterator<Item = (N, C)>, entries: &[E]) -> Model
where
    N: AsRef<std::path::Path>,
    C: Into<Vec<u8>>,
    E: AsRef<std::path::Path>,
{
    let mut l = memory_loader(files);
    l.set_base_dir(load::MEMORY_DIR);
    let dir = std::path::Path::new(load::MEMORY_DIR);
    for e in entries {
        l.load_file(dir.join(e));
    }
    l.build()
}

fn memory_loader<N, C>(files: impl IntoIterator<Item = (N, C)>) -> Loader
where
    N: AsRef<std::path::Path>,
    C: Into<Vec<u8>>,
{
    let mut l = Loader::default();
    l.set_read_files(false);
    let dir = std::path::Path::new(load::MEMORY_DIR);
    for (n, c) in files {
        l.add_file(dir.join(n), c);
    }
    l
}

/// Loads the given entry files (e.g. usage model, allocation, monitor repository) and everything
/// they reference, with default options.
pub fn load_files<P: AsRef<std::path::Path>>(files: &[P]) -> Model {
    let mut l = Loader::default();
    for f in files {
        l.load_file(f);
    }
    l.build()
}
