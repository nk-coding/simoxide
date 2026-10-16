//! Environments: per module (Eclipse's `QvtOperationalModuleEnv` with its type resolver's
//! additional operations), and the variable scopes of the code being analysed (the operation
//! environment and its children; Eclipse's `AbstractEnvironment` variable lists).

use crate::ir::{self, Direction, GlobalId, IntermPropId, LocalId, ModelParamRef, ModelTypeId, ModuleId, OpId};
use crate::ops::OpRef;
use qvto_syntax::Span;
use qvto_types::{StdOp, TypeId};

/// A module's environment and facts.
pub(crate) struct ModuleEnv {
    pub ir: ir::Module,
    pub name: String,
    pub is_library: bool,
    /// The unit that declares it (`usize::MAX` for `Stdlib`).
    pub unit: usize,
    /// Environments imported by access (`fByAccess`, unique, in order).
    pub by_access: Vec<ModuleId>,
    /// Environments imported by extension (`fByExtension`): `Stdlib` first.
    pub by_extension: Vec<ModuleId>,
    /// `QvtTypeResolverImpl.fCtx2OperationMap`: contextual operations by context type.
    pub ctx_ops: Vec<(TypeId, Vec<OpId>)>,
    /// `fAdditionalTypes`
    pub additional_types: Vec<TypeId>,
    pub defines_ocl_any_features: bool,
    /// Non-imperative additional operations by owner (the library's `defineOperation`s).
    pub std_additional: Vec<(TypeId, Vec<StdOp>)>,
    /// `myModelTypeRegistry` (name to model type), in registration order.
    pub model_types: Vec<(String, ModelTypeId)>,
    /// The module class's own operations (`getEOperations`), in definition order.
    pub eoperations: Vec<OpRef>,
    /// Module features (`getEStructuralFeatures`): properties, configuration properties,
    /// contextual properties.
    pub features: Vec<ModuleFeature>,
    pub model_params: Vec<ModelParamInfo>,
    /// `registerMappingOperation`: mappings by (owner, name).
    pub mappings_map: Vec<((TypeId, String), Vec<OpId>)>,
    /// `rename` aliases: (new name, owner type, original feature name).
    pub aliases: Vec<(String, TypeId, String)>,
    /// The `extends` imports (`ModuleImport` of kind EXTENSION), for module-level overriding.
    pub module_imports: Vec<(ModuleId, bool)>,
    pub ty: TypeId,
    pub header_span: Span,
    pub blackbox: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleFeature {
    Global(GlobalId),
    Interm(IntermPropId),
}

#[derive(Clone, Debug)]
pub(crate) struct ModelParamInfo {
    pub name: String,
    pub direction: Direction,
    /// `None` when the parameter's type is not a model type (an error was reported).
    pub model_type: Option<ModelTypeId>,
    pub ty: TypeId,
    pub span: Span,
}

pub(crate) struct GlobalInfo {
    pub ir: ir::Global,
    pub hidden: bool,
    /// Span of the property's name (problem node).
    pub name_span: Span,
    pub is_config: bool,
    pub changeable: bool,
}

pub(crate) struct IntermPropInfo {
    pub ir: ir::IntermProp,
    pub context: Option<qvto_model::ClassId>,
    pub hidden: bool,
    pub name_span: Span,
}

/// A variable reference.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum VarRef {
    Local(LocalId),
    This(ModuleId),
    ModelParam(ModelParamRef),
}

/// What Eclipse's `Variable.getRepresentedParameter()` tells: a parameter of an operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParamInfo {
    pub direction: Direction,
    /// A `MappingParameter` (parameters of mappings and of `main`).
    pub mapping_param: bool,
    pub extent: Option<ModelParamRef>,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalExtra {
    pub param: Option<ParamInfo>,
    /// The variable is the context parameter `self` of a mapping or helper.
    pub is_context: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct VarEntry {
    pub name: String,
    pub var: VarRef,
    pub explicit: bool,
}

/// One Eclipse environment level.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scope {
    pub vars: Vec<VarEntry>,
    pub self_var: Option<VarRef>,
    /// `setParentLocal`: `lookupLocal` continues in the parent.
    pub parent_local: bool,
}

/// The code body being analysed.
#[derive(Default)]
pub(crate) struct FnState {
    pub locals: Vec<ir::Local>,
    pub extra: Vec<LocalExtra>,
    /// Environment levels, outermost (the module's) first.
    pub scopes: Vec<Scope>,
    /// The operation being analysed (`getContextOperation`).
    pub op: Option<OpId>,
    /// `isWithinInitMappingSection`
    pub in_init: bool,
    /// `AbstractEnvironment.generateName` counter (implicit variables).
    pub name_counter: u32,
    /// Variables Eclipse would add to the module's resource etc.: none needed.
    pub module: Option<ModuleId>,
}

impl FnState {
    pub fn new_local(&mut self, name: &str, ty: TypeId, kind: ir::LocalKind, span: Span) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(ir::Local {
            name: name.to_string(),
            ty,
            kind,
            span,
        });
        self.extra.push(LocalExtra {
            param: None,
            is_context: false,
        });
        id
    }

    pub fn top(&mut self) -> &mut Scope {
        self.scopes.last_mut().expect("scope")
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }
}
