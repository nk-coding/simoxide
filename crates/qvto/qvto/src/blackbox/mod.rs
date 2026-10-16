//! Blackboxes: native libraries the host provides, as Eclipse provides Java blackbox units.
//!
//! A blackbox library has two halves:
//!
//! - **Signatures** for the analyser: a [`BlackboxUnit`] per unit (qualified name, e.g.
//!   `org.bar.Foo` or `Strings`), with its modules, the metamodels they use and each
//!   operation's kind, context, parameters, result and implementation key, as Eclipse derives
//!   them from a Java class. They are data: [`Catalog::parse`] reads them in the tab-separated
//!   form `qvto-types/data/blackboxes.tsv` uses (dumped from Eclipse for the corpus' Java
//!   classes); a host writes the same rows for its own libraries.
//! - **Implementations** for the VM: Rust functions in a [`Natives`] registry, by
//!   implementation key (for Java libraries `pkg.Class#method(java.lang.String, ...)`) or, for
//!   `blackbox transformation`s, by transformation name.
//!
//! The units are made visible to the analyser through a [`UnitResolver`]
//! ([`BlackboxResolver`] puts them behind a file resolver: registered units, then units found
//! "on the classpath"), the implementations to a run with `Vm::set_natives`.
//!
//! [`stdlib`] holds QVTo's own deprecated libraries (`Strings`, `emf.tools`) with their
//! signatures and implementations.

pub mod java;
pub mod stdlib;

pub use qvto_sema::{BlackboxModule, BlackboxOp, BlackboxOpKind, BlackboxParam, BlackboxUnit};
pub use qvto_vm::{NativeCall, NativeError, NativeFn, NativeResult, Natives, java_method_key};

use qvto_sema::ir::Direction;
use qvto_sema::{UnitResolver, UnitSource};
use std::collections::HashMap;
use std::rc::Rc;

/// A registered unit: qualified name, file name (last URI segment), signatures.
#[derive(Clone, Debug)]
pub struct CatalogUnit {
    pub qualified_name: String,
    pub file_name: String,
    pub unit: Rc<BlackboxUnit>,
}

impl CatalogUnit {
    /// The unit as the analyser sees it (key `qvto://blackbox/<name>`, no text).
    pub fn source(&self) -> UnitSource {
        UnitSource {
            key: format!("qvto://blackbox/{}", self.qualified_name),
            qualified_name: self.qualified_name.clone(),
            file_name: self.file_name.clone(),
            path: None,
            text: Rc::from(""),
            blackbox: Some(self.unit.clone()),
        }
    }
}

/// Blackbox signatures by environment (a set of registrations, e.g. one test case's
/// `BlackboxRegistry.registerModule` calls) and the cases that use each environment.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub envs: HashMap<String, Vec<CatalogUnit>>,
    /// Case id -> environment of its own registrations.
    pub case_env: HashMap<String, String>,
}

fn parse_direction(s: &str) -> Direction {
    match s {
        "out" => Direction::Out,
        "inout" => Direction::InOut,
        _ => Direction::In,
    }
}

fn parse_kind(s: &str) -> BlackboxOpKind {
    match s {
        "query" => BlackboxOpKind::Query,
        "mapping" => BlackboxOpKind::Mapping,
        "constructor" => BlackboxOpKind::Constructor,
        _ => BlackboxOpKind::Helper,
    }
}

impl Catalog {
    /// Reads the signature table. Rows (tab-separated):
    ///
    /// ```text
    /// env     <env> <kind> <registrations> <case ids, comma-separated | ->
    /// unit    <env> <qualified name> <uri> OK|ERROR [<message>]
    /// module  <env> <unit> <module name> Library|Transformation <java class> <nsURI,...>
    /// op      <env> <unit> <module> <name> helper|query|mapping|constructor <context | -> <static>
    ///         <name:dir:type;...> <result | -> <implementation key> <flags: hidden,javaStatic,withExecutionContext>
    /// ```
    ///
    /// Types are written `oclstdlib::String`, `Sequence(oclstdlib::T)`, `<nsURI>#<Class>`.
    pub fn parse(text: &str) -> Result<Catalog, String> {
        let mut cat = Catalog::default();
        // (env, unit) -> index into envs[env]
        let mut index: HashMap<(String, String), usize> = HashMap::new();
        let mut units: HashMap<(String, String), BlackboxUnit> = HashMap::new();
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or("");
            match f[0] {
                "env" => {
                    cat.envs.entry(get(1).to_string()).or_default();
                    if get(4) != "-" {
                        for id in get(4).split(',').filter(|s| !s.is_empty()) {
                            cat.case_env.insert(id.to_string(), get(1).to_string());
                        }
                    }
                }
                "unit" => {
                    let uri = get(3);
                    let file_name = uri.rsplit('/').next().unwrap_or(uri).to_string();
                    let env = cat.envs.entry(get(1).to_string()).or_default();
                    index.insert((get(1).to_string(), get(2).to_string()), env.len());
                    env.push(CatalogUnit {
                        qualified_name: get(2).to_string(),
                        file_name,
                        unit: Rc::new(BlackboxUnit::default()),
                    });
                    units.insert(
                        (get(1).to_string(), get(2).to_string()),
                        BlackboxUnit {
                            modules: Vec::new(),
                            load_error: get(4) != "OK",
                        },
                    );
                }
                "module" => {
                    let m = BlackboxModule {
                        name: get(3).to_string(),
                        library: get(4) == "Library",
                        metamodels: get(6)
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect(),
                        ops: Vec::new(),
                    };
                    units
                        .get_mut(&(get(1).to_string(), get(2).to_string()))
                        .ok_or_else(|| format!("module before its unit: {line}"))?
                        .modules
                        .push(m);
                }
                "op" => {
                    let params = get(8)
                        .split(';')
                        .filter(|s| !s.is_empty())
                        .map(|p| {
                            let mut it = p.splitn(3, ':');
                            BlackboxParam {
                                name: it.next().unwrap_or("").to_string(),
                                direction: parse_direction(it.next().unwrap_or("in")),
                                ty: it.next().unwrap_or("").to_string(),
                            }
                        })
                        .collect();
                    let flags: Vec<&str> = get(11)
                        .split([',', ' '])
                        .filter(|s| !s.is_empty())
                        .collect();
                    let op = BlackboxOp {
                        name: get(4).to_string(),
                        kind: parse_kind(get(5)),
                        context: (get(6) != "-" && !get(6).is_empty()).then(|| get(6).to_string()),
                        params,
                        result: get(9).to_string(),
                        hidden: flags.contains(&"hidden"),
                        java_static: flags.contains(&"javaStatic"),
                        with_context: flags.contains(&"withExecutionContext"),
                        java_method: get(10).to_string(),
                    };
                    units
                        .get_mut(&(get(1).to_string(), get(2).to_string()))
                        .and_then(|u| u.modules.iter_mut().find(|m| m.name == get(3)))
                        .ok_or_else(|| format!("op before its module: {line}"))?
                        .ops
                        .push(op);
                }
                _ => {}
            }
        }
        for ((env, unit), u) in units {
            let i = index[&(env.clone(), unit)];
            cat.envs.get_mut(&env).expect("env")[i].unit = Rc::new(u);
        }
        Ok(cat)
    }

    /// The units of an environment.
    pub fn units(&self, env: &str) -> &[CatalogUnit] {
        self.envs.get(env).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Adds the units of another catalog (same environment names merge).
    pub fn extend(&mut self, other: &Catalog) {
        for (k, v) in &other.envs {
            self.envs.entry(k.clone()).or_default().extend(v.iter().cloned());
        }
        for (k, v) in &other.case_env {
            self.case_env.insert(k.clone(), v.clone());
        }
    }
}

/// A unit resolver with blackbox units: the inner resolver first (files), then the registered
/// units (Eclipse's `BlackboxRegistry`), then the units found on the "classpath" (Java classes
/// by name).
pub struct BlackboxResolver<'r> {
    pub inner: &'r dyn UnitResolver,
    /// Registered units, in lookup order; they are also the candidates for `blackbox`
    /// operations declared in QVTo modules that import no blackbox unit.
    pub registered: Vec<CatalogUnit>,
    /// Units found by class name only.
    pub classpath: Vec<CatalogUnit>,
}

impl UnitResolver for BlackboxResolver<'_> {
    fn resolve(&self, importer: &UnitSource, qualified_name: &str) -> Option<UnitSource> {
        if let Some(u) = self.inner.resolve(importer, qualified_name) {
            return Some(u);
        }
        self.registered
            .iter()
            .chain(self.classpath.iter())
            .find(|u| u.qualified_name == qualified_name)
            .map(CatalogUnit::source)
    }

    fn blackbox_units(&self) -> Vec<UnitSource> {
        let mut units: Vec<&CatalogUnit> = self.registered.iter().collect();
        units.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
        units.dedup_by(|a, b| a.qualified_name == b.qualified_name);
        units.into_iter().map(CatalogUnit::source).collect()
    }
}
