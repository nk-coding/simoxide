//! Operations as the analyser sees them (Eclipse's `EOperation`s): standard library operations
//! (with the owner list they were found in, as each classic OCL type has its own operation
//! objects), the program's imperative operations, and metamodel operations.

use crate::analyzer::Sema;
use crate::env::ModuleFeature;
use crate::ir::{self, GlobalId, IntermPropId, ModuleId, OpId};
use qvto_model::FeatureId;
use qvto_syntax::Span;
use qvto_types::{Section, StdOp, StdOwner, Type, TypeId};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OpRef {
    Std(StdOp, StdOwner),
    User(OpId),
    Meta(ir::MetaOpId),
}

/// An imperative operation of the program with its signature as Eclipse records it.
pub(crate) struct OpInfo {
    pub ir: ir::Operation,
    /// The owning classifier: the context type, or the module type.
    pub owner: TypeId,
    pub contextual: bool,
    /// `getEParameters` (name, type, direction).
    pub params: Vec<(String, TypeId, ir::Direction)>,
    /// `getEType`
    pub ret: TypeId,
    pub module: ModuleId,
    pub overridden: Option<OpId>,
    /// `HiddenElementAdapter.markAsHidden`
    pub hidden: bool,
    pub is_query: bool,
    /// Problem nodes: the method's name node (`getMethodNameProblemNodeCS`) and header node
    /// (`getMethodHeaderProblemNodeCS`), the declaration (`MappingDeclarationCS`).
    pub name_node: Span,
    pub header_node: Span,
    pub decl_span: Span,
    /// The mapping's virtual table, built during collision checks.
    pub vtable: Vec<OpId>,
}

/// An analysed argument as the overload resolution sees it.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Arg {
    pub ty: TypeId,
    /// The argument is a type expression (`TypeExp`).
    pub type_exp: bool,
    /// The referred type of a type expression.
    pub referred: Option<TypeId>,
}

/// A property (Eclipse's `EStructuralFeature`): a metamodel feature, a module property, a
/// contextual intermediate property, or a tuple part.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PropRef {
    Feature(FeatureId),
    Global(GlobalId),
    Interm(IntermPropId),
    /// A part of a tuple type by index.
    TuplePart(TypeId, u32),
}

impl From<ModuleFeature> for PropRef {
    fn from(f: ModuleFeature) -> PropRef {
        match f {
            ModuleFeature::Global(g) => PropRef::Global(g),
            ModuleFeature::Interm(i) => PropRef::Interm(i),
        }
    }
}

impl<'r> Sema<'r> {
    pub fn op_name(&self, o: OpRef) -> &str {
        match o {
            OpRef::Std(op, _) => op.name(),
            OpRef::User(id) => &self.ops[id.index()].ir.name,
            OpRef::Meta(_) => "",
        }
    }

    /// Parameter types (`getEParameters().getEType()`).
    pub fn op_params(&self, o: OpRef) -> Vec<TypeId> {
        match o {
            OpRef::Std(op, _) => self.stdlib.params(op).to_vec(),
            OpRef::User(id) => self.ops[id.index()].params.iter().map(|p| p.1).collect(),
            OpRef::Meta(_) => Vec::new(),
        }
    }

    pub fn op_param_names(&self, o: OpRef) -> Vec<String> {
        match o {
            OpRef::Std(op, _) => op.desc().params.iter().map(|p| p.0.to_string()).collect(),
            OpRef::User(id) => self.ops[id.index()].params.iter().map(|p| p.0.clone()).collect(),
            OpRef::Meta(_) => Vec::new(),
        }
    }

    /// `getEType`
    pub fn op_ret(&self, o: OpRef) -> TypeId {
        match o {
            OpRef::Std(op, _) => self.stdlib.ret(op),
            OpRef::User(id) => self.ops[id.index()].ret,
            OpRef::Meta(_) => TypeId::OCL_VOID,
        }
    }

    /// `uml.getOwningClassifier(operation)`.
    pub fn op_owner(&mut self, o: OpRef) -> TypeId {
        match o {
            OpRef::Std(_, StdOwner::Stdlib) => self.modules[crate::analyzer::STDLIB_MODULE.index()].ty,
            OpRef::Std(_, owner) => self.stdlib.owner_type(&mut self.ts.types, owner),
            OpRef::User(id) => self.ops[id.index()].owner,
            OpRef::Meta(_) => TypeId::OCL_VOID,
        }
    }

    /// The module owning an operation (`QvtOperationalParserUtil.getOwningModule`): user
    /// operations' module, `Stdlib` for QVTo's library operations, none for classic OCL's and
    /// metamodel operations.
    pub fn op_module(&self, o: OpRef) -> Option<ModuleId> {
        match o {
            OpRef::Std(op, _) => match op.desc().section {
                Section::Ocl => None,
                _ => Some(crate::analyzer::STDLIB_MODULE),
            },
            OpRef::User(id) => Some(self.ops[id.index()].module),
            OpRef::Meta(_) => None,
        }
    }

    pub fn op_is_imperative(&self, o: OpRef) -> bool {
        matches!(o, OpRef::User(_))
    }

    pub fn op_is_mapping(&self, o: OpRef) -> bool {
        matches!(o, OpRef::User(id) if self.ops[id.index()].ir.kind == ir::OpKind::Mapping)
    }

    pub fn op_is_constructor(&self, o: OpRef) -> bool {
        matches!(o, OpRef::User(id) if self.ops[id.index()].ir.kind == ir::OpKind::Constructor)
    }

    pub fn op_is_static(&self, o: OpRef) -> bool {
        match o {
            OpRef::Std(op, _) => op.desc().is_static,
            _ => false,
        }
    }

    pub fn op_overridden(&self, o: OpRef) -> Option<OpRef> {
        match o {
            OpRef::User(id) => self.ops[id.index()].overridden.map(OpRef::User),
            _ => None,
        }
    }

    /// The context type of an imperative operation.
    pub fn op_context(&self, o: OpRef) -> Option<TypeId> {
        match o {
            OpRef::User(id) => {
                let op = &self.ops[id.index()];
                op.contextual.then_some(op.owner)
            }
            _ => None,
        }
    }

    /// `FormattingHelperImpl.formatOperationSignature`: `owner::name(T1, T2) : R`.
    pub fn format_op_signature(&mut self, o: OpRef) -> String {
        let owner = self.op_owner(o);
        let mut s = String::new();
        let on = self.type_name(owner);
        s.push_str(&on);
        s.push_str("::");
        s.push_str(self.op_name(o));
        s.push('(');
        let params = self.op_params(o);
        let names = self.op_param_names(o);
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&names[i]);
            s.push_str(" : ");
            let pn = self.type_name(*p);
            s.push_str(&pn);
        }
        s.push(')');
        let r = self.op_ret(o);
        if r != TypeId::OCL_VOID {
            s.push_str(" : ");
            let rn = self.type_name(r);
            s.push_str(&rn);
        }
        s
    }
}

impl<'r> Sema<'r> {
    /// The owning classifier of a property.
    pub fn prop_owner(&mut self, p: PropRef) -> TypeId {
        match p {
            PropRef::Feature(f) => {
                let c = self.ts.meta.feature(f).owner;
                self.ts.types.class(c)
            }
            PropRef::Global(g) => {
                let m = self.globals[g.index()].ir.module;
                self.modules[m.index()].ty
            }
            PropRef::Interm(i) => {
                let p = &self.interm_props[i.index()];
                match p.context {
                    Some(c) => self.ts.types.class(c),
                    None => TypeId::OCL_VOID,
                }
            }
            PropRef::TuplePart(t, _) => t,
        }
    }

    pub fn prop_name(&self, p: PropRef) -> String {
        match p {
            PropRef::Feature(f) => self.ts.meta.feature(f).name.to_string(),
            PropRef::Global(g) => self.globals[g.index()].ir.name.clone(),
            PropRef::Interm(i) => self.interm_props[i.index()].ir.name.clone(),
            PropRef::TuplePart(t, i) => match self.ts.types.get(t) {
                Type::Tuple(parts) => parts[i as usize].0.to_string(),
                _ => String::new(),
            },
        }
    }

    /// `uml.getOCLType(property)`: the property's type, a collection for many-valued metamodel
    /// features.
    pub fn prop_type(&mut self, p: PropRef) -> TypeId {
        match p {
            PropRef::Feature(f) => self.feature_type(f),
            PropRef::Global(g) => self.globals[g.index()].ir.ty,
            PropRef::Interm(i) => self.interm_props[i.index()].ir.ty,
            PropRef::TuplePart(t, i) => match self.ts.types.get(t) {
                Type::Tuple(parts) => parts[i as usize].1,
                _ => TypeId::OCL_VOID,
            },
        }
    }

    /// The OCL type of a metamodel feature (`UMLReflectionImpl.getOCLType`): many-valued
    /// features (upper bound > 1 or < 0) are collections by `ordered`/`unique`.
    pub fn feature_type(&mut self, f: FeatureId) -> TypeId {
        let fi = self.ts.meta.feature(f);
        let many = fi.upper > 1 || fi.upper < 0;
        let (ordered, unique) = (fi.ordered, fi.unique);
        let elem = self.classifier_type(fi.ty);
        let elem = self.ts.ck().ocl_type(elem);
        if many {
            let kind = qvto_types::CollKind::of_feature(ordered, unique);
            self.ts.types.collection(kind, elem)
        } else {
            elem
        }
    }

    pub fn classifier_type(&mut self, c: qvto_model::Classifier) -> TypeId {
        match c {
            qvto_model::Classifier::Class(c) => self.ts.types.class(c),
            qvto_model::Classifier::DataType(d) => self.ts.types.intern(Type::DataType(d)),
            qvto_model::Classifier::Enum(e) => self.ts.types.intern(Type::Enum(e)),
        }
    }

    pub fn prop_changeable(&self, p: PropRef) -> bool {
        match p {
            PropRef::Feature(f) => self.ts.meta.feature(f).changeable,
            PropRef::Global(g) => self.globals[g.index()].changeable,
            _ => true,
        }
    }
}
