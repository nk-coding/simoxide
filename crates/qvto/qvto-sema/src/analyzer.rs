//! The analyser's state: the program under construction, the environments of all modules, the
//! units, and the per-body state of the code being analysed.

use crate::diag::Problems;
use crate::env::{FnState, ModuleEnv};
use crate::ir::{self, ModuleId, OpId, Program};
use crate::ops::OpInfo;
use crate::units::{Compilation, DepElem, UnitResolver, UnitResult, UnitState};
use qvto_model::{ClassId, MetaRegistry};
use qvto_syntax::Span;
use qvto_types::check::{Checker, ModelTypeInfo, ModuleTypeInfo};
use qvto_types::{StdClasses, StdLib, TypeId, TypeTable};
use std::collections::HashMap;

/// Types and the facts the type checker needs.
pub(crate) struct TypeState {
    pub types: TypeTable,
    pub meta: MetaRegistry,
    pub std: StdClasses,
    pub module_infos: Vec<ModuleTypeInfo>,
    pub model_type_infos: Vec<ModelTypeInfo>,
}

impl TypeState {
    pub fn ck(&mut self) -> Checker<'_> {
        Checker {
            types: &mut self.types,
            meta: &self.meta,
            std: &self.std,
            modules: &self.module_infos,
            model_types: &self.model_type_infos,
        }
    }
}

/// A model type (`modeltype M uses ...`).
#[derive(Clone, Debug)]
pub(crate) struct ModelTypeState {
    pub decl: ir::ModelTypeDecl,
    /// `strict` compliance kind given.
    pub span: Span,
}

pub(crate) struct Sema<'r> {
    pub resolver: &'r dyn UnitResolver,
    pub ts: TypeState,
    pub stdlib: StdLib,
    pub units: Vec<UnitState>,
    pub unit_index: HashMap<String, usize>,
    pub dep_path: Vec<DepElem>,
    pub cur_unit: usize,
    /// All modules; `ModuleId(0)` is QVTo's library module `Stdlib`.
    pub modules: Vec<ModuleEnv>,
    pub ops: Vec<OpInfo>,
    pub globals: Vec<crate::env::GlobalInfo>,
    pub model_types: Vec<ModelTypeState>,
    pub interm_props: Vec<crate::env::IntermPropInfo>,
    pub interm_classes: Vec<ir::IntermClass>,
    pub sources: Vec<ir::Source>,
    /// The code body being analysed.
    pub fun: FnState,
    /// The module whose environment is current.
    pub cur_module: ModuleId,
    /// Classes declared as intermediate classes or exceptions, and their module.
    pub interm_class_module: HashMap<ClassId, ModuleId>,
    /// Late resolves seen in the current unit (validated after each module's bodies).
    pub late_resolves: Vec<crate::imp::LateResolve>,
    /// Per operation: what Eclipse knows about its parameter variables.
    pub op_extras: Vec<Vec<crate::env::LocalExtra>>,
    /// Declared OCL types of intermediate class features (the registry keeps a classifier).
    pub interm_feature_types: HashMap<qvto_model::FeatureId, TypeId>,
    /// Package of placeholder data types for intermediate features of non-classifier types.
    pub placeholder_pkg: Option<qvto_model::PackageId>,
    /// Source references of mapping extensions per mapping: inherits, disjuncts, merges
    /// (`MappingExtensionHelper.bind2SourceElement`).
    pub ext_refs: HashMap<OpId, [Vec<Span>; 3]>,
    /// Metamodel operations (`EOperation`s) and their parameter names.
    pub meta_ops: Vec<ir::MetaOp>,
    pub meta_op_names: Vec<Vec<String>>,
    /// Metamodel operations by owning class, in `eOperations` order.
    pub class_ops: HashMap<ClassId, Vec<ir::MetaOpId>>,
    /// The source type for "Cannot find operation" messages of implicit collects.
    pub call_msg_src: Option<TypeId>,
    /// Tuple types in creation order (`TypeResolver` cache).
    pub tuple_types: Vec<TypeId>,
}

pub(crate) const STDLIB_MODULE: ModuleId = ModuleId(0);

impl<'r> Sema<'r> {
    pub fn new(resolver: &'r dyn UnitResolver, mut meta: MetaRegistry) -> Sema<'r> {
        let std = StdClasses::register(&mut meta);
        let mut types = TypeTable::new();
        let stdlib = StdLib::new(&mut types, std);
        let mut s = Sema {
            resolver,
            ts: TypeState {
                types,
                meta,
                std,
                module_infos: Vec::new(),
                model_type_infos: Vec::new(),
            },
            stdlib,
            units: Vec::new(),
            unit_index: HashMap::new(),
            dep_path: Vec::new(),
            cur_unit: 0,
            modules: Vec::new(),
            ops: Vec::new(),
            globals: Vec::new(),
            model_types: Vec::new(),
            interm_props: Vec::new(),
            interm_classes: Vec::new(),
            sources: Vec::new(),
            fun: FnState::default(),
            cur_module: STDLIB_MODULE,
            interm_class_module: HashMap::new(),
            late_resolves: Vec::new(),
            op_extras: Vec::new(),
            interm_feature_types: HashMap::new(),
            placeholder_pkg: None,
            ext_refs: HashMap::new(),
            meta_ops: Vec::new(),
            meta_op_names: Vec::new(),
            class_ops: HashMap::new(),
            call_msg_src: None,
            tuple_types: Vec::new(),
        };
        s.create_stdlib_module();
        s
    }

    #[inline]
    pub fn problems(&mut self) -> &mut Problems {
        &mut self.units[self.cur_unit].problems
    }

    pub fn error(&mut self, span: Span, msg: impl Into<String>) {
        let u = self.cur_unit;
        self.units[u].problems.error(span, msg);
    }

    pub fn warning(&mut self, span: Span, msg: impl Into<String>) {
        let u = self.cur_unit;
        self.units[u].problems.warning(span, msg);
    }

    /// The source text of the current unit.
    pub fn src(&self) -> std::rc::Rc<str> {
        self.units[self.cur_unit].source.text.clone()
    }

    pub fn module_name(&self, m: ModuleId) -> &str {
        &self.modules[m.index()].name
    }

    /// The current module's name (prefix of the full names of collection and tuple types).
    pub fn cur_module_name(&self) -> String {
        self.modules[self.cur_module.index()].name.clone()
    }

    /// `QvtOperationalTypesUtil.getTypeFullName` in the current module.
    pub fn full_name(&mut self, t: TypeId) -> String {
        let m = self.cur_module_name();
        self.ts.ck().full_name(t, &m)
    }

    /// `uml.getName(type)`.
    pub fn type_name(&mut self, t: TypeId) -> String {
        self.ts.ck().name(t)
    }

    /// `safeGetQualifiedName`.
    pub fn qualified_name(&mut self, t: TypeId) -> String {
        let m = self.cur_module_name();
        self.ts.ck().qualified_name(t, &m)
    }

    /// `TypeResolver.resolveTupleType`: an existing tuple type with the same part names whose
    /// part types are the same OCL types (`EString` and `String` alike) is reused.
    pub fn tuple_type(&mut self, parts: Vec<(Box<str>, TypeId)>) -> TypeId {
        let key: Vec<(Box<str>, TypeId)> = parts.iter().map(|(n, t)| (n.clone(), self.ts.ck().ocl_type(*t))).collect();
        for i in 0..self.tuple_types.len() {
            let t = self.tuple_types[i];
            let qvto_types::Type::Tuple(existing) = self.ts.types.get(t).clone() else {
                continue;
            };
            if existing.len() != key.len() {
                continue;
            }
            let mut all = true;
            for (n, kt) in &key {
                match existing.iter().find(|(en, _)| en == n) {
                    Some((_, et)) => {
                        let eo = self.ts.ck().ocl_type(*et);
                        if eo != *kt {
                            all = false;
                            break;
                        }
                    }
                    None => {
                        all = false;
                        break;
                    }
                }
            }
            if all {
                return t;
            }
        }
        let t = self.ts.types.tuple(parts);
        if !self.tuple_types.contains(&t) {
            self.tuple_types.push(t);
        }
        t
    }

    pub fn op(&self, o: OpId) -> &OpInfo {
        &self.ops[o.index()]
    }

    pub fn op_mut(&mut self, o: OpId) -> &mut OpInfo {
        &mut self.ops[o.index()]
    }

    /// Assembles the result.
    pub fn finish(mut self, main: usize) -> Compilation {
        let mut units = Vec::new();
        // main first, then the others in compilation order
        let mut order: Vec<usize> = vec![main];
        order.extend((0..self.units.len()).filter(|&i| i != main));
        let remap: HashMap<usize, usize> = order.iter().enumerate().map(|(n, &o)| (o, n)).collect();
        for &i in &order {
            let u = &mut self.units[i];
            let problems = std::mem::take(&mut u.problems).into_vec();
            units.push(UnitResult {
                source: u.source.clone(),
                lines: u.lines.clone(),
                problems,
                imports: u.imports.iter().map(|x| remap[x]).collect(),
            });
        }
        let main_module = self.units[main].modules.first().copied().unwrap_or(STDLIB_MODULE);
        let program = self.into_program(main_module);
        Compilation { units, program }
    }

    fn into_program(self, main_module: ModuleId) -> Program {
        Program {
            types: self.ts.types,
            meta: self.ts.meta,
            std_classes: self.ts.std,
            sources: self.sources,
            modules: self.modules.into_iter().map(|m| m.ir).collect(),
            main_module,
            model_types: self.model_types.into_iter().map(|m| m.decl).collect(),
            operations: self.ops.into_iter().map(|o| o.ir).collect(),
            globals: self.globals.into_iter().map(|g| g.ir).collect(),
            interm_props: self.interm_props.into_iter().map(|p| p.ir).collect(),
            interm_classes: self.interm_classes,
            meta_ops: self.meta_ops,
        }
    }
}
