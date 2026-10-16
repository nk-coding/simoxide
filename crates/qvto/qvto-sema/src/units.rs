//! Compilation units and imports: a port of `QVTOCompiler.doCompile` (imports compiled first,
//! depth first, with cycle detection and per-unit caching), and of the parser's
//! `setupTopLevel`, which distributes the top-level elements of a file over its modules.

use crate::analyzer::Sema;
use crate::diag::{Problem, Problems};
use crate::ir::Program;
use crate::messages::{bind, compiler as cm};
use qvto_model::MetaRegistry;
use qvto_syntax::ast;
use qvto_syntax::{LineIndex, Span};
use std::path::PathBuf;
use std::rc::Rc;

/// A compilation unit's source.
#[derive(Clone, Debug)]
pub struct UnitSource {
    /// Unique key of the unit (its canonical path or URI), for caching and cycle detection.
    pub key: String,
    /// Qualified name, `a.b.C` for `a/b/C.qvto` below its source root.
    pub qualified_name: String,
    /// File name as Eclipse prints it (last URI segment).
    pub file_name: String,
    pub path: Option<PathBuf>,
    pub text: Rc<str>,
    /// A Java blackbox unit (no text): its modules and signatures.
    pub blackbox: Option<Rc<crate::blackbox::BlackboxUnit>>,
}

impl UnitSource {
    /// The namespace part of the qualified name (`a.b` of `a.b.C`), if any.
    pub fn namespace(&self) -> Option<&str> {
        self.qualified_name.rsplit_once('.').map(|(n, _)| n)
    }
}

/// Finds imported units (Eclipse's `UnitResolver`).
pub trait UnitResolver {
    /// The unit with the given qualified name, as seen from `importer`.
    fn resolve(&self, importer: &UnitSource, qualified_name: &str) -> Option<UnitSource>;

    /// The Java blackbox units registered in the compilation's context (Eclipse's
    /// `BlackboxRegistry`), searched for implementations of `blackbox` operations declared in
    /// QVTo modules that import no blackbox unit.
    fn blackbox_units(&self) -> Vec<UnitSource> {
        Vec::new()
    }
}

/// The result of compiling one unit.
#[derive(Clone, Debug)]
pub struct UnitResult {
    pub source: UnitSource,
    pub lines: LineIndex,
    /// Syntax and semantic problems, errors first.
    pub problems: Vec<Problem>,
    /// Indexes of the directly imported units in [`Compilation::units`].
    pub imports: Vec<usize>,
}

impl UnitResult {
    pub fn errors(&self) -> impl Iterator<Item = &Problem> {
        self.problems.iter().filter(|p| p.is_error())
    }
}

/// The result of a compilation: every unit (the main unit first) and the program.
#[derive(Clone, Debug)]
pub struct Compilation {
    pub units: Vec<UnitResult>,
    pub program: Program,
}

impl Compilation {
    pub fn main(&self) -> &UnitResult {
        &self.units[0]
    }

    /// Errors of the main unit and everything it imports (`TransformationUtil.getErrors`).
    pub fn has_errors(&self) -> bool {
        self.units.iter().any(|u| u.errors().next().is_some())
    }
}

/// Compiles a unit and everything it imports. `meta` must hold the metamodels the units refer
/// to by namespace URI (Eclipse's package registry), Ecore included.
pub fn compile(main: UnitSource, resolver: &dyn UnitResolver, meta: MetaRegistry) -> Compilation {
    compile_with_operations(main, resolver, meta, &[])
}

/// Operations of metamodel classes (`EOperation`s): the registry has none; the host supplies
/// them.
pub use qvto_model::{MetaMultiplicity, MetaOperation};

/// [`compile`] with the metamodels' operations, in `eOperations` order per class.
pub fn compile_with_operations(
    main: UnitSource,
    resolver: &dyn UnitResolver,
    meta: MetaRegistry,
    ops: &[MetaOperation],
) -> Compilation {
    let mut sema = Sema::new(resolver, meta);
    sema.add_meta_operations(ops);
    let main_idx = sema.compile_unit(main);
    sema.finish(main_idx)
}

impl<'r> Sema<'r> {
    fn add_meta_operations(&mut self, ops: &[MetaOperation]) {
        for o in ops {
            let mut params = Vec::new();
            let mut names = Vec::new();
            for (n, c, m) in &o.params {
                let t = self.meta_typed(*c, *m);
                params.push(t);
                names.push(n.clone());
            }
            let result = match &o.result {
                Some((c, m)) => self.meta_typed(*c, *m),
                None => qvto_types::TypeId::OCL_VOID,
            };
            let id = crate::ir::MetaOpId(self.meta_ops.len() as u32);
            self.meta_ops.push(crate::ir::MetaOp {
                class: o.class,
                name: o.name.clone(),
                params,
                result,
            });
            self.meta_op_names.push(names);
            self.class_ops.entry(o.class).or_default().push(id);
        }
    }

    /// `UMLReflectionImpl.getOCLType` of a typed element.
    fn meta_typed(&mut self, c: qvto_model::Classifier, m: MetaMultiplicity) -> qvto_types::TypeId {
        let t = self.classifier_type(c);
        let t = self.ts.ck().ocl_type(t);
        if m.many {
            let k = qvto_types::CollKind::of_feature(m.ordered, m.unique);
            self.ts.types.collection(k, t)
        } else {
            t
        }
    }
}

/// The namespace URIs of the model types of a unit and everything it imports, for a host that
/// loads metamodels on demand before compiling.
pub fn scan_metamodel_uris(main: &UnitSource, resolver: &dyn UnitResolver) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![main.clone()];
    while let Some(u) = stack.pop() {
        if !seen.insert(u.key.clone()) {
            continue;
        }
        let parsed = qvto_syntax::parse(&u.text);
        let visit = |e: &ast::Element, out: &mut Vec<String>, stack: &mut Vec<UnitSource>| match e {
            ast::Element::ModelType(m) => {
                for p in &m.packages {
                    if let Some(uri) = &p.uri
                        && !out.contains(&uri.value)
                    {
                        out.push(uri.value.clone());
                    }
                }
            }
            ast::Element::Import(i) => {
                let q = i
                    .unit
                    .iter()
                    .map(|n| n.text.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                if let Some(s) = resolve_import(resolver, &u, &q) {
                    stack.push(s);
                }
            }
            _ => {}
        };
        for e in &parsed.unit.elements {
            visit(e, &mut out, &mut stack);
            if let ast::Element::Module(m) = e {
                for b in m.body.iter().flatten() {
                    visit(b, &mut out, &mut stack);
                }
            }
        }
    }
    out
}

/// `QVTOCompiler.resolveImportedUnit`: the name as is, then within the importer's namespace.
fn resolve_import(
    resolver: &dyn UnitResolver,
    importer: &UnitSource,
    q: &str,
) -> Option<UnitSource> {
    if let Some(u) = resolver.resolve(importer, q) {
        return Some(u);
    }
    if let Some(ns) = importer.namespace()
        && !q.contains('.')
    {
        return resolver.resolve(importer, &format!("{ns}.{q}"));
    }
    None
}

/// One compiled unit inside the analyser.
pub(crate) struct UnitState {
    pub source: UnitSource,
    pub lines: LineIndex,
    pub problems: Problems,
    pub imports: Vec<usize>,
    /// Modules of the unit (`CompiledUnit.getModuleEnvironments`).
    pub modules: Vec<crate::ir::ModuleId>,
    /// Imported units by qualified name segments (`UnitResolverImpl`).
    pub imported: Vec<(Vec<String>, usize)>,
    /// The file environment's package registry: metamodels resolved by URI in this unit.
    pub registry: Vec<(String, qvto_model::PackageId)>,
}

/// An element of the dependency walk (`DependencyPathElement`).
pub(crate) struct DepElem {
    pub key: String,
    /// The import being processed (its path span) and the importer's unit.
    pub current_import: Option<Span>,
    pub unit: Option<usize>,
}

/// A module with the elements `setupTopLevel` gives it.
pub(crate) struct ModuleCs<'u> {
    pub module: &'u ast::Module,
    pub metamodels: Vec<&'u ast::ModelType>,
    pub imports: Vec<&'u ast::Import>,
    pub classifiers: Vec<&'u ast::ClassDef>,
    pub properties: Vec<&'u ast::Property>,
    pub methods: Vec<MethodCs<'u>>,
    pub renamings: Vec<&'u ast::Rename>,
    pub tags: Vec<&'u ast::Tag>,
}

#[derive(Copy, Clone)]
pub(crate) enum MethodCs<'u> {
    Helper(&'u ast::Helper),
    Entry(&'u ast::Entry),
    Mapping(&'u ast::Mapping),
    Constructor(&'u ast::Constructor),
}

impl<'u> MethodCs<'u> {
    pub fn span(&self) -> Span {
        match self {
            MethodCs::Helper(h) => h.span,
            MethodCs::Entry(e) => e.span,
            MethodCs::Mapping(m) => m.span,
            MethodCs::Constructor(c) => c.span,
        }
    }
}

/// `AbstractQVTParser.setupTopLevel` and `initializeModule`.
pub(crate) fn setup_top_level(unit: &ast::Unit) -> Vec<ModuleCs<'_>> {
    let mut modules = Vec::new();
    let mut modeltypes = Vec::new();
    let mut classifiers = Vec::new();
    let mut properties = Vec::new();
    let mut helpers = Vec::new();
    let mut mappings = Vec::new();
    let mut constructors = Vec::new();
    let mut renamings = Vec::new();
    let mut imports = Vec::new();
    let mut tags = Vec::new();
    for e in &unit.elements {
        match e {
            ast::Element::Module(m) => modules.push(m),
            ast::Element::ModelType(m) => modeltypes.push(m),
            ast::Element::Class(c) => classifiers.push(c),
            ast::Element::Property(p) => properties.push(p),
            ast::Element::Helper(h) => helpers.push(MethodCs::Helper(h)),
            ast::Element::Entry(h) => helpers.push(MethodCs::Entry(h)),
            ast::Element::Mapping(m) => mappings.push(MethodCs::Mapping(m)),
            ast::Element::Constructor(c) => constructors.push(MethodCs::Constructor(c)),
            ast::Element::Rename(r) => renamings.push(r),
            ast::Element::Import(i) => imports.push(i),
            ast::Element::Tag(t) => tags.push(t),
            ast::Element::Error(_) => {}
        }
    }
    let single = modules.len() == 1;
    let mut out = Vec::new();
    for m in modules {
        let mut cs = ModuleCs {
            module: m,
            metamodels: modeltypes.clone(),
            imports: imports.clone(),
            classifiers: Vec::new(),
            properties: Vec::new(),
            methods: Vec::new(),
            renamings: Vec::new(),
            tags: Vec::new(),
        };
        // initializeModule: the body's elements grouped by kind
        let mut b_helpers = Vec::new();
        let mut b_mappings = Vec::new();
        let mut b_ctors = Vec::new();
        for e in m.body.iter().flatten() {
            match e {
                ast::Element::Class(c) => cs.classifiers.push(c),
                ast::Element::Property(p) => cs.properties.push(p),
                ast::Element::Helper(h) => b_helpers.push(MethodCs::Helper(h)),
                ast::Element::Entry(h) => b_helpers.push(MethodCs::Entry(h)),
                ast::Element::Mapping(x) => b_mappings.push(MethodCs::Mapping(x)),
                ast::Element::Constructor(c) => b_ctors.push(MethodCs::Constructor(c)),
                ast::Element::Tag(t) => cs.tags.push(t),
                // the grammar has no other module elements
                _ => {}
            }
        }
        cs.methods.extend(b_helpers);
        cs.methods.extend(b_mappings);
        cs.methods.extend(b_ctors);
        if single {
            cs.properties.extend(properties.iter().copied());
            cs.methods.extend(helpers.iter().copied());
            cs.methods.extend(mappings.iter().copied());
            cs.methods.extend(constructors.iter().copied());
            cs.renamings.extend(renamings.iter().copied());
            cs.classifiers.extend(classifiers.iter().copied());
            cs.tags.extend(tags.iter().copied());
        }
        out.push(cs);
    }
    out
}

/// All imports of a unit as `QvtOperationalParserUtil.getImports` lists them: the imports of
/// each module (the top-level ones copied into each).
pub(crate) fn unit_imports<'u>(modules: &[ModuleCs<'u>]) -> Vec<&'u ast::Import> {
    let mut v = Vec::new();
    for m in modules {
        v.extend(m.imports.iter().copied());
    }
    v
}

pub(crate) fn path_string(names: &[ast::Name], sep: &str) -> String {
    names
        .iter()
        .map(|n| n.text.as_str())
        .collect::<Vec<_>>()
        .join(sep)
}

pub(crate) fn path_span(names: &[ast::Name]) -> Span {
    match (names.first(), names.last()) {
        (Some(a), Some(b)) => a.span.to(b.span),
        _ => Span::default(),
    }
}

impl<'r> Sema<'r> {
    /// `QVTOCompiler.doCompile`.
    pub(crate) fn compile_unit(&mut self, source: UnitSource) -> usize {
        self.dep_path.push(DepElem {
            key: source.key.clone(),
            current_import: None,
            unit: None,
        });
        let r = self.compile_unit_inner(source);
        self.dep_path.pop();
        r
    }

    fn compile_unit_inner(&mut self, source: UnitSource) -> usize {
        if let Some(&u) = self.unit_index.get(&source.key) {
            return u;
        }
        if let Some(bb) = source.blackbox.clone() {
            return self.compile_blackbox_unit(source, &bb);
        }
        let text = source.text.clone();
        let parsed = qvto_syntax::parse(&text);
        let lines = LineIndex::new(&text);
        let mut problems = Problems::default();
        for d in &parsed.diagnostics {
            if d.is_error() {
                problems.error(d.span, d.message.clone());
            } else {
                problems.warning(d.span, d.message.clone());
            }
        }
        let uidx = self.units.len();
        self.units.push(UnitState {
            source: source.clone(),
            lines,
            problems,
            imports: Vec::new(),
            modules: Vec::new(),
            imported: Vec::new(),
            registry: Vec::new(),
        });
        self.unit_index.insert(source.key.clone(), uidx);
        if let Some(d) = self.dep_path.last_mut() {
            d.unit = Some(uidx);
        }
        let modules = setup_top_level(&parsed.unit);
        let imports = unit_imports(&modules);
        for imp in &imports {
            let q = path_string(&imp.unit, ".");
            if q.is_empty() {
                continue;
            }
            let pspan = path_span(&imp.unit);
            let Some(imported) = resolve_import(self.resolver, &source, &q) else {
                let msg = bind(cm::importedCompilationUnitNotFound, &[&q]);
                self.units[uidx].problems.error(pspan, msg);
                continue;
            };
            if let Some(d) = self.dep_path.last_mut() {
                d.current_import = Some(pspan);
            }
            // cycle: the imported unit is on the walk path
            if let Some(pos) = self.dep_path.iter().position(|d| d.key == imported.key) {
                let importer_elem = &self.dep_path[pos];
                if importer_elem.unit != Some(uidx)
                    && let (Some(iu), Some(ispan)) =
                        (importer_elem.unit, importer_elem.current_import)
                {
                    let msg = bind(
                        cm::cyclicImportError,
                        &[&self.unit_uri(iu), &self.unit_uri_of(&source)],
                    );
                    self.units[iu].problems.error(ispan, msg);
                }
                let msg = bind(
                    cm::cyclicImportError,
                    &[&self.unit_uri_of(&source), &self.uri_of_source(&imported)],
                );
                self.units[uidx].problems.error(pspan, msg);
                continue;
            }
            let ci = self.compile_unit(imported);
            if self.units[ci].problems.has_errors() {
                let msg = bind(
                    cm::importHasCompilationError,
                    &[&path_string(&imp.unit, "::")],
                );
                self.units[uidx].problems.error(pspan, msg);
            }
            if !self.units[uidx].imports.contains(&ci) {
                self.units[uidx].imports.push(ci);
            }
            let segs: Vec<String> = imp.unit.iter().map(|n| n.text.clone()).collect();
            let unit = &mut self.units[uidx];
            if let Some(e) = unit.imported.iter_mut().find(|(s, _)| *s == segs) {
                e.1 = ci;
            } else {
                unit.imported.push((segs, ci));
            }
        }
        self.cur_unit = uidx;
        if !modules.is_empty() {
            self.analyze_unit(uidx, &modules, &text);
        }
        self.check_dup_imports(uidx, &imports);
        uidx
    }

    fn compile_blackbox_unit(
        &mut self,
        source: UnitSource,
        bb: &crate::blackbox::BlackboxUnit,
    ) -> usize {
        let mut problems = Problems::default();
        if bb.load_error {
            let msg = bind(
                crate::messages::validation::FailedToLoadUnit,
                &[&source.qualified_name],
            );
            let msg = bind(
                crate::messages::validation::QvtOperationalVisitorCS_SeeErrorLogForDetails,
                &[&msg],
            );
            problems.error(Span::new(0, 0), msg);
        }
        let uidx = self.units.len();
        self.units.push(UnitState {
            source: source.clone(),
            lines: LineIndex::new(""),
            problems,
            imports: Vec::new(),
            modules: Vec::new(),
            imported: Vec::new(),
            registry: Vec::new(),
        });
        self.unit_index.insert(source.key.clone(), uidx);
        while self.sources.len() < uidx + 2 {
            self.sources.push(crate::ir::Source {
                name: String::new(),
                qualified_name: String::new(),
                path: None,
                lines: LineIndex::new(""),
            });
        }
        self.sources[uidx + 1] = crate::ir::Source {
            name: source.file_name.clone(),
            qualified_name: source.qualified_name.clone(),
            path: None,
            lines: LineIndex::new(""),
        };
        let saved = (self.cur_unit, self.cur_module);
        self.cur_unit = uidx;
        self.load_blackbox_unit(uidx, bb);
        self.cur_unit = saved.0;
        self.cur_module = saved.1;
        uidx
    }

    /// The URI Eclipse prints for a unit in cycle messages (`file:` URI of its path, or its key).
    fn unit_uri(&self, u: usize) -> String {
        self.unit_uri_of(&self.units[u].source)
    }

    fn unit_uri_of(&self, s: &UnitSource) -> String {
        self.uri_of_source(s)
    }

    fn uri_of_source(&self, s: &UnitSource) -> String {
        match &s.path {
            Some(p) => format!("file:{}", p.display()),
            None => s.key.clone(),
        }
    }

    /// `QVTOCompiler.checkForDupImports`: a second import of an already imported unit at another
    /// position is a warning.
    fn check_dup_imports(&mut self, u: usize, imports: &[&ast::Import]) {
        if imports.len() < 2 {
            return;
        }
        let mut checked_units = Vec::new();
        let mut checked_starts = Vec::new();
        let mut dups = Vec::new();
        for imp in imports {
            let q: Vec<String> = imp.unit.iter().map(|n| n.text.clone()).collect();
            let target = self.units[u]
                .imported
                .iter()
                .find(|(s, _)| *s == q)
                .map(|(_, i)| *i)
                .filter(|&i| !self.units[i].modules.is_empty());
            if let Some(t) = target
                && checked_units.contains(&t)
                && !checked_starts.contains(&imp.span.start)
            {
                dups.push(*imp);
                continue;
            }
            checked_units.push(target.unwrap_or(usize::MAX));
            checked_starts.push(imp.span.start);
        }
        for d in dups {
            let msg = bind(
                cm::compilationUnitAlreadyImported,
                &[&path_string(&d.unit, ".")],
            );
            self.units[u].problems.warning(path_span(&d.unit), msg);
        }
    }
}
