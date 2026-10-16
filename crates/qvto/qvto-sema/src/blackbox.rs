//! Java blackbox units (Eclipse's `BlackboxUnitResolver`): the host describes the modules and
//! operation signatures Eclipse derives from the Java classes; the analyser makes library
//! modules of them whose operations are blackbox operations.

use crate::analyzer::Sema;
use crate::env::ParamInfo;
use crate::ir::{self, Direction, ModuleId, OpId};
use crate::ops::{OpInfo, OpRef};
use qvto_syntax::Span;
use qvto_types::{CollKind, Type, TypeId};

/// A blackbox unit as Eclipse loads it from Java.
#[derive(Clone, Debug, Default)]
pub struct BlackboxUnit {
    pub modules: Vec<BlackboxModule>,
    /// Eclipse failed to load (part of) the unit; the unit then has the error
    /// `Failed to load unit '<name>'. See error log for details`.
    pub load_error: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BlackboxModule {
    pub name: String,
    /// `Library` (always, for Java libraries) or a transformation.
    pub library: bool,
    /// Namespace URIs of the metamodels the module uses.
    pub metamodels: Vec<String>,
    pub ops: Vec<BlackboxOp>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BlackboxOpKind {
    #[default]
    Helper,
    Query,
    Mapping,
    Constructor,
}

#[derive(Clone, Debug, Default)]
pub struct BlackboxOp {
    pub name: String,
    pub kind: BlackboxOpKind,
    /// Context type (signature notation: `oclstdlib::String`, `Set(oclstdlib::T)`,
    /// `<nsURI>#<Name>`), `None` for module operations.
    pub context: Option<String>,
    pub params: Vec<BlackboxParam>,
    pub result: String,
    /// Hidden from the analyser (`@Operation(hidden)`).
    pub hidden: bool,
    /// The Java method is static (`javaStatic`).
    pub java_static: bool,
    /// The Java method takes the execution context first (`withExecutionContext`).
    pub with_context: bool,
    /// The Java method, `pkg.Class#name(java.lang.String, ...)`.
    pub java_method: String,
}

#[derive(Clone, Debug)]
pub struct BlackboxParam {
    pub name: String,
    pub direction: Direction,
    pub ty: String,
}

impl<'r> Sema<'r> {
    /// Creates the modules of a blackbox unit.
    pub(crate) fn load_blackbox_unit(&mut self, u: usize, bb: &BlackboxUnit) {
        for bm in &bb.modules {
            let id = self.new_module_pub(&bm.name, bm.library, u, Span::default());
            self.units[u].modules.push(id);
            self.cur_module = id;
            for uri in &bm.metamodels {
                if let Some(p) = self.ts.meta.package_by_ns_uri(uri) {
                    self.units[u].registry.push((uri.clone(), p));
                }
            }
            for op in &bm.ops {
                self.blackbox_op(id, op);
            }
        }
    }

    fn blackbox_op(&mut self, module: ModuleId, b: &BlackboxOp) {
        let module_ty = self.modules[module.index()].ty;
        let context = b
            .context
            .as_deref()
            .filter(|c| *c != "-")
            .map(|c| self.parse_bb_type(c));
        let mut locals = Vec::new();
        let mut extras = Vec::new();
        let mut params = Vec::new();
        let mut sig = Vec::new();
        let mapping = b.kind == BlackboxOpKind::Mapping;
        for p in &b.params {
            let ty = self.parse_bb_type(&p.ty);
            let local = ir::LocalId(locals.len() as u32);
            locals.push(ir::Local {
                name: p.name.clone(),
                ty,
                kind: ir::LocalKind::Param,
                span: Span::default(),
            });
            extras.push(crate::env::LocalExtra {
                param: Some(ParamInfo {
                    direction: p.direction,
                    mapping_param: mapping,
                    extent: None,
                }),
                is_context: false,
            });
            sig.push((p.name.clone(), ty, p.direction));
            params.push(ir::Param {
                name: p.name.clone(),
                direction: p.direction,
                ty,
                extent: None,
                local,
                span: Span::default(),
            });
        }
        let mut self_local = None;
        if let Some(ct) = context {
            let l = ir::LocalId(locals.len() as u32);
            locals.push(ir::Local {
                name: "self".into(),
                ty: ct,
                kind: ir::LocalKind::SelfVar,
                span: Span::default(),
            });
            extras.push(crate::env::LocalExtra {
                param: Some(ParamInfo {
                    direction: Direction::In,
                    mapping_param: mapping,
                    extent: None,
                }),
                is_context: true,
            });
            self_local = Some(l);
        }
        let ret = if b.result.is_empty() || b.result == "-" {
            TypeId::OCL_VOID
        } else {
            self.parse_bb_type(&b.result)
        };
        let mut results = Vec::new();
        let mut result_local = None;
        if ret != TypeId::OCL_VOID && b.kind != BlackboxOpKind::Constructor {
            let l = ir::LocalId(locals.len() as u32);
            locals.push(ir::Local {
                name: "result".into(),
                ty: ret,
                kind: ir::LocalKind::Result,
                span: Span::default(),
            });
            extras.push(crate::env::LocalExtra {
                param: Some(ParamInfo {
                    direction: Direction::Out,
                    mapping_param: mapping,
                    extent: None,
                }),
                is_context: false,
            });
            results.push(ir::Param {
                name: "result".into(),
                direction: Direction::Out,
                ty: ret,
                extent: None,
                local: l,
                span: Span::default(),
            });
            result_local = Some(l);
        }
        let kind = match b.kind {
            BlackboxOpKind::Helper => ir::OpKind::Helper,
            BlackboxOpKind::Query => ir::OpKind::Query,
            BlackboxOpKind::Mapping => ir::OpKind::Mapping,
            BlackboxOpKind::Constructor => ir::OpKind::Constructor,
        };
        let id = OpId(self.ops.len() as u32);
        let source = self.modules[module.index()].ir.source;
        self.ops.push(OpInfo {
            ir: ir::Operation {
                module,
                kind,
                name: b.name.clone(),
                context,
                context_direction: Direction::In,
                params,
                results,
                result_ty: ret,
                is_abstract: false,
                is_blackbox: true,
                is_static: b.java_static && context.is_none(),
                locals,
                self_local,
                result_local,
                body: None,
                mapping: None,
                vtable: Vec::new(),
                overrides: None,
                source,
                decl_span: Span::default(),
                span: Span::default(),
                blackbox_impl: Some(b.java_method.clone()),
            },
            owner: context.unwrap_or(module_ty),
            contextual: context.is_some(),
            params: sig,
            ret,
            module,
            overridden: None,
            hidden: b.hidden,
            is_query: b.kind == BlackboxOpKind::Query,
            name_node: Span::default(),
            header_node: Span::default(),
            decl_span: Span::default(),
            vtable: Vec::new(),
        });
        self.op_extras.push(extras);
        if context.is_some() {
            let me = &mut self.modules[module.index()];
            let owner = context.unwrap();
            if owner == TypeId::OCL_ANY {
                me.defines_ocl_any_features = true;
            }
            if let Some(e) = me.ctx_ops.iter_mut().find(|(t, _)| *t == owner) {
                e.1.push(id);
            } else {
                me.ctx_ops.push((owner, vec![id]));
            }
            if !me.additional_types.contains(&owner) {
                me.additional_types.push(owner);
            }
        }
        let me = &mut self.modules[module.index()];
        me.eoperations.push(OpRef::User(id));
        me.ir.operations.push(id);
    }

    /// Parses a blackbox signature type: the library's notation plus `<nsURI>#<Name>` for
    /// metamodel classifiers.
    pub(crate) fn parse_bb_type(&mut self, s: &str) -> TypeId {
        let s = s.trim();
        if let Some((uri, name)) = s.rsplit_once('#') {
            if !uri.contains('(') {
                return match self
                    .ts
                    .meta
                    .package_by_ns_uri(uri)
                    .and_then(|p| self.ts.meta.classifier_by_name(p, name))
                {
                    Some(c) => {
                        let t = self.classifier_type(c);
                        self.ts.ck().ocl_type(t)
                    }
                    None => TypeId::OCL_VOID,
                };
            }
        }
        if let Some((head, rest)) = s.split_once('(') {
            let inner = rest.strip_suffix(')').unwrap_or(rest);
            let args = split_args(inner);
            let kind = match head {
                "Collection" => Some(CollKind::Collection),
                "Set" => Some(CollKind::Set),
                "OrderedSet" => Some(CollKind::OrderedSet),
                "Bag" => Some(CollKind::Bag),
                "Sequence" => Some(CollKind::Sequence),
                _ => None,
            };
            if let Some(k) = kind {
                let e = self.parse_bb_type(args[0]);
                return self.ts.types.collection(k, e);
            }
            return match head {
                "List" => {
                    let e = self.parse_bb_type(args[0]);
                    self.ts.types.list(e)
                }
                "Dict" if args.len() == 2 => {
                    let k = self.parse_bb_type(args[0]);
                    let v = self.parse_bb_type(args[1]);
                    self.ts.types.dict(k, v)
                }
                "TypeType" => {
                    let e = self.parse_bb_type(args[0]);
                    self.ts.types.type_type(e)
                }
                "Tuple" => {
                    let mut parts = Vec::new();
                    for a in args {
                        if let Some((n, t)) = a.split_once(':') {
                            let tt = self.parse_bb_type(t);
                            parts.push((n.into(), tt));
                        }
                    }
                    self.tuple_type(parts)
                }
                _ => TypeId::OCL_VOID,
            };
        }
        let name = s.strip_prefix("oclstdlib::").unwrap_or(s);
        match name {
            "OclAny" => TypeId::OCL_ANY,
            "OclVoid" => TypeId::OCL_VOID,
            "OclInvalid" => TypeId::OCL_INVALID,
            "Boolean" => TypeId::BOOLEAN,
            "Integer" => TypeId::INTEGER,
            "Real" => TypeId::REAL,
            "String" => TypeId::STRING,
            "UnlimitedNatural" => TypeId::UNLIMITED_NATURAL,
            "OclElement" => TypeId::OCL_ELEMENT,
            "T" => TypeId::T,
            "T2" => TypeId::T2,
            "Stdlib::KeyT" => TypeId::KEY_T,
            "Stdlib::CommonT" => TypeId::COMMON_T,
            other => {
                let n = other.strip_prefix("Stdlib::").unwrap_or(other);
                match self.ts.std.by_name(n) {
                    Some(c) => self.ts.types.class(c),
                    None => self.ts.types.intern(Type::OclAny),
                }
            }
        }
    }
}

fn split_args(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}
