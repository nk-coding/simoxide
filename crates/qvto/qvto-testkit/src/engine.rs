//! The interface an engine implements to run under the harness, and the stub used until the
//! engine exists.

use crate::case::Case;
use crate::fixture::Problem;
use qvto_emf::ResourceSet;
use qvto_emf::config::Config;
use qvto_model::ResourceId;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Severity of an EMF diagnostic, with EMF's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok = 0,
    Info = 1,
    Warning = 2,
    Error = 4,
    Cancel = 8,
}

impl Severity {
    pub fn from_name(s: &str) -> Option<Severity> {
        Some(match s {
            "OK" => Severity::Ok,
            "INFO" => Severity::Info,
            "WARNING" => Severity::Warning,
            "ERROR" => Severity::Error,
            "CANCEL" => Severity::Cancel,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Severity::Ok => "OK",
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
            Severity::Cancel => "CANCEL",
        }
    }
}

/// What the harness hands an engine for one case: the case, the repository root its paths are
/// relative to, and the metamodel configuration shared with the Java oracle.
pub struct CaseEnv<'a> {
    pub case: &'a Case,
    pub base: &'a Path,
    pub models: &'a Config,
}

impl CaseEnv<'_> {
    /// A path of the case (relative to its directory) under the repository root.
    pub fn path(&self, p: &str) -> PathBuf {
        self.case.path(self.base, p)
    }

    pub fn transformation(&self) -> Option<PathBuf> {
        self.case.qvto.as_deref().map(|q| self.path(q))
    }

    /// A resource set set up as the Java runner sets up the case's (metamodel index and URI maps
    /// of the configuration, the case's `mmfile`s registered, its `urimap`s), for loading the
    /// inputs. Generated and dynamic packages of the Eclipse bundles come from their `.ecore`
    /// files through the configuration's index.
    pub fn resource_set(&self) -> Result<ResourceSet, String> {
        let anchor = self.transformation().unwrap_or_else(|| self.base.join("x"));
        let mut rs = self.models.resource_set_for(&anchor);
        for (from, to) in &self.case.urimaps {
            let abs = std::path::absolute(self.base.join(to)).map_err(|e| e.to_string())?;
            let mut u = qvto_emf::uri::Uri::file(&abs.to_string_lossy()).to_string();
            if !u.ends_with('/') {
                u.push('/');
            }
            rs.map_uri(from, &u);
        }
        if !self.case.mmfiles.is_empty() {
            // TestQvtStandaloneExecutor maps the workspace URI of Ecore.ecore to Ecore's resource
            rs.add_alias("platform:/resource/org.eclipse.emf.ecore/model/Ecore.ecore", "http://www.eclipse.org/emf/2002/Ecore");
        }
        for mm in &self.case.mmfiles {
            let p = self.path(mm);
            let r = rs.load_file(&p)?;
            rs.register(r)?;
        }
        Ok(rs)
    }
}

/// How far an engine got with a case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The engine cannot run transformations yet.
    NotImplemented(String),
    /// The engine does not support something the case needs (a Java blackbox, a feature).
    Unsupported(String),
    /// The engine failed (panic, internal error): a bug, not a QVTo result.
    Crashed(String),
    /// The engine ran the case (successfully or with a QVTo-level failure, see the outcome).
    Completed,
}

/// The out/inout extents after a run: each in a resource of `rs` with the URI
/// `.../extent<i>.xmi` (i = parameter index), in parameter order, as the Java runner dumps them.
/// For cases with `dump models`, `models` holds every other model resource (inputs and the
/// models they reference, all proxies resolved, no `.ecore` files) sorted by file name.
pub struct Outputs {
    pub rs: ResourceSet,
    pub extents: Vec<ResourceId>,
    pub models: Vec<ResourceId>,
}

impl Outputs {
    /// The resources the canonical dump covers, in order.
    pub fn dumped(&self) -> Vec<ResourceId> {
        self.extents.iter().chain(self.models.iter()).copied().collect()
    }

    /// The other model resources of `rs` for `dump models`: file resources that are not
    /// extents, `.ecore` files or excluded, sorted by file name (call after `resolve_all`).
    pub fn collect_models(rs: &ResourceSet, extents: &[ResourceId], exclude: &[String]) -> Vec<ResourceId> {
        let mut v: Vec<(String, ResourceId)> = rs
            .graph
            .resource_ids()
            .filter(|r| !extents.contains(r))
            .filter_map(|r| {
                let uri = &rs.graph.resource(r).uri;
                if !uri.starts_with("file:") || uri.ends_with(".ecore") || exclude.contains(uri) {
                    return None;
                }
                Some((qvto_emf::canon::label(uri), r))
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, r)| r).collect()
    }
}

/// Everything observable of one run, in the Java runner's terms.
pub struct Outcome {
    pub status: Status,
    /// Severity of loading (compiling) the transformation.
    pub load: Option<Severity>,
    /// Compile problems of the unit and its imports.
    pub problems: Vec<Problem>,
    /// Severity and code of the execution diagnostic.
    pub severity: Option<Severity>,
    pub code: i32,
    pub message: String,
    /// Java class name of the terminating QVT exception (`org.eclipse.m2m.internal.qvt.oml.evaluator.QvtRuntimeException`, ...).
    pub exception: Option<String>,
    /// The printed QVT stack trace of the terminating exception (`QvtRuntimeException.printQvtStackTrace`).
    pub stack: Option<String>,
    /// The transformation's log, in `WriterLog`'s format.
    pub log: String,
    pub trace_records: Option<usize>,
    pub outputs: Option<Outputs>,
    pub compile_time: Option<Duration>,
    pub run_time: Option<Duration>,
}

impl Outcome {
    pub fn with_status(status: Status) -> Outcome {
        Outcome {
            status,
            load: None,
            problems: Vec::new(),
            severity: None,
            code: 0,
            message: String::new(),
            exception: None,
            stack: None,
            log: String::new(),
            trace_records: None,
            outputs: None,
            compile_time: None,
            run_time: None,
        }
    }

    pub fn errors(&self) -> usize {
        self.problems.iter().filter(|p| p.error).count()
    }

    pub fn warnings(&self) -> usize {
        self.problems.iter().filter(|p| !p.error).count()
    }
}

/// A QVTo engine under test. The harness calls [`Engine::run`] once per runnable case.
///
/// Contract (mirrors the Java runner, `reference/runner/src/qvtorun/CaseRunner.java`):
/// - compile `env.transformation()`; imports resolve against the case's bundle source
///   containers, or the transformation's folder when `case.imports_from_folder`;
/// - for compile-only cases (`case.compile_only()`) return after compiling, with `load` and
///   `problems`;
/// - bind the inputs positionally to the in/inout parameters (loaded with
///   `env.resource_set()`), configuration properties from `case.config`, blackboxes from
///   `case.blackboxes`, session options;
/// - run, collect the log, the execution diagnostic, the terminating exception and its stack;
/// - put each out/inout extent into a resource `extent<i>.xmi` (parameter index i) of the
///   result's resource set ([`Outputs`]).
pub trait Engine {
    fn name(&self) -> &str;
    fn run(&mut self, env: &CaseEnv) -> Outcome;
}

/// The engine until Q3: reports every case as not implemented, so the pipeline runs end to end.
pub struct StubEngine;

impl Engine for StubEngine {
    fn name(&self) -> &str {
        "stub"
    }

    fn run(&mut self, _env: &CaseEnv) -> Outcome {
        Outcome::with_status(Status::NotImplemented("qvto-rs cannot run transformations yet (Q3)".into()))
    }
}
