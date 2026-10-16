//! A small builder of typed IR programs for VM tests (the analyser's output, written by hand).

#![allow(dead_code)]

use qvto_emf::Graph;
use qvto_model::{
    ClassId, Classifier, FeatureId, FeatureKind, MetaRegistry, ModelStore, PackageId,
};
use qvto_sema::ir::*;
use qvto_syntax::{LineIndex, Span};
use qvto_types::{CollKind, Section, StdClasses, StdOp, StdOwner, Type, TypeId, TypeTable, stdlib};
use qvto_vm::{RunOptions, RunOutcome, Vm};

pub struct Irb {
    pub p: Program,
    pub graph: Graph,
    pub entry: OpId,
}

pub fn sp(at: u32) -> Span {
    Span::new(at, at + 1)
}

thread_local! {
    static POS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The source offset of the expressions built from now on.
pub fn set_pos(offset: u32) {
    POS.with(|p| p.set(offset));
}

pub fn e(kind: ExprKind, ty: TypeId) -> Expr {
    Expr {
        kind,
        ty,
        span: sp(POS.with(|p| p.get())),
    }
}

pub fn int(i: i64) -> Expr {
    e(ExprKind::Int(i), TypeId::INTEGER)
}
pub fn real(x: f64) -> Expr {
    e(ExprKind::Real(x), TypeId::REAL)
}
pub fn s(x: &str) -> Expr {
    e(ExprKind::Str(x.into()), TypeId::STRING)
}
pub fn b(x: bool) -> Expr {
    e(ExprKind::Bool(x), TypeId::BOOLEAN)
}
pub fn null() -> Expr {
    e(ExprKind::Null, TypeId::OCL_VOID)
}
pub fn at(mut x: Expr, offset: u32) -> Expr {
    x.span = sp(offset);
    x
}

/// A standard library operation by owner and name (OCL section first, then QVTo's).
pub fn sop(owner: StdOwner, name: &str, nparams: usize) -> StdOp {
    for sec in [Section::Ocl, Section::Qvto, Section::Class] {
        for &op in stdlib::owner_ops(sec, owner) {
            if op.name() == name && op.desc().params.len() == nparams {
                return op;
            }
        }
    }
    panic!("no std op {owner:?}::{name}/{nparams}")
}

pub fn std(op: StdOp, src: Option<Expr>, args: Vec<Expr>, ty: TypeId) -> Expr {
    e(
        ExprKind::Call(Box::new(Call {
            callee: Callee::Std(op),
            source: src,
            args,
        })),
        ty,
    )
}

pub fn bin(owner: StdOwner, name: &str, a: Expr, x: Expr, ty: TypeId) -> Expr {
    std(sop(owner, name, 1), Some(a), vec![x], ty)
}

pub fn log(msg: Expr) -> Expr {
    e(
        ExprKind::Log(Box::new(Log {
            message: Some(msg),
            element: None,
            level: None,
            when: None,
        })),
        TypeId::OCL_VOID,
    )
}

pub fn block(v: Vec<Expr>) -> Expr {
    e(ExprKind::Block(v), TypeId::OCL_VOID)
}

impl Irb {
    pub fn new(src: &str) -> Irb {
        let graph = Graph::new();
        let mut meta: MetaRegistry = graph.meta().clone();
        let std_classes = StdClasses::register(&mut meta);
        let mut types = TypeTable::new();
        let mty = types.intern(Type::Module(0));
        let p = Program {
            types,
            meta,
            std_classes,
            sources: vec![Source {
                name: "test.qvto".into(),
                qualified_name: "test".into(),
                path: None,
                lines: LineIndex::new(src),
            }],
            modules: vec![Module {
                name: "test".into(),
                kind: ModuleKind::Transformation,
                blackbox: false,
                source: SourceId(0),
                span: sp(0),
                ty: mty,
                model_params: Vec::new(),
                model_types: Vec::new(),
                usages: Vec::new(),
                globals: Vec::new(),
                operations: Vec::new(),
                entry: None,
                classes: Vec::new(),
            }],
            main_module: ModuleId(0),
            model_types: Vec::new(),
            operations: Vec::new(),
            globals: Vec::new(),
            interm_props: Vec::new(),
            interm_classes: Vec::new(),
            meta_ops: Vec::new(),
        };
        let mut irb = Irb {
            p,
            graph,
            entry: OpId(0),
        };
        let main = irb.op("main", OpKind::Entry, None, TypeId::OCL_VOID);
        irb.p.modules[0].entry = Some(main);
        irb.entry = main;
        irb
    }

    pub fn ty(&mut self, t: Type) -> TypeId {
        self.p.types.intern(t)
    }

    pub fn coll(&mut self, k: CollKind, el: TypeId) -> TypeId {
        self.p.types.collection(k, el)
    }

    pub fn op(
        &mut self,
        name: &str,
        kind: OpKind,
        context: Option<TypeId>,
        result: TypeId,
    ) -> OpId {
        let id = OpId(self.p.operations.len() as u32);
        self.p.operations.push(Operation {
            module: ModuleId(0),
            kind,
            name: name.into(),
            context,
            context_direction: Direction::In,
            params: Vec::new(),
            results: Vec::new(),
            result_ty: result,
            is_abstract: false,
            is_blackbox: false,
            is_static: false,
            locals: Vec::new(),
            self_local: None,
            result_local: None,
            body: Some(Vec::new()),
            mapping: if kind == OpKind::Mapping {
                Some(MappingInfo::default())
            } else {
                None
            },
            vtable: Vec::new(),
            overrides: None,
            source: SourceId(0),
            decl_span: sp(0),
            span: sp(0),
        });
        if kind == OpKind::Mapping {
            self.p.operations[id.index()].body = None;
        }
        self.p.modules[0].operations.push(id);
        if let Some(c) = context {
            let l = self.local(id, "self", c, LocalKind::SelfVar);
            self.p.operations[id.index()].self_local = Some(l);
        }
        id
    }

    pub fn local(&mut self, op: OpId, name: &str, ty: TypeId, kind: LocalKind) -> LocalId {
        let o = &mut self.p.operations[op.index()];
        o.locals.push(Local {
            name: name.into(),
            ty,
            kind,
            span: sp(0),
        });
        LocalId(o.locals.len() as u32 - 1)
    }

    pub fn var(&mut self, op: OpId, name: &str, ty: TypeId) -> LocalId {
        self.local(op, name, ty, LocalKind::Var)
    }

    pub fn param(&mut self, op: OpId, name: &str, ty: TypeId, dir: Direction) -> LocalId {
        let l = self.local(op, name, ty, LocalKind::Param);
        self.p.operations[op.index()].params.push(Param {
            name: name.into(),
            direction: dir,
            ty,
            extent: None,
            local: l,
            span: sp(0),
        });
        l
    }

    /// The single result parameter (`result`).
    pub fn result(&mut self, op: OpId, ty: TypeId) -> LocalId {
        let l = self.local(op, "result", ty, LocalKind::Result);
        let o = &mut self.p.operations[op.index()];
        o.results.push(Param {
            name: "result".into(),
            direction: Direction::Out,
            ty,
            extent: None,
            local: l,
            span: sp(0),
        });
        o.result_local = Some(l);
        o.result_ty = ty;
        l
    }

    pub fn body(&mut self, op: OpId, stmts: Vec<Expr>) {
        self.p.operations[op.index()].body = Some(stmts);
    }

    pub fn mapping(&mut self, op: OpId) -> &mut MappingInfo {
        self.p.operations[op.index()].mapping.as_mut().unwrap()
    }

    pub fn main(&mut self, stmts: Vec<Expr>) {
        let m = self.entry;
        self.body(m, stmts);
    }

    /// A metamodel package with classes, registered in the store and the program.
    pub fn package(&mut self, name: &str) -> PackageId {
        let uri = format!("http://test/{name}");
        let p = self
            .graph
            .meta_mut()
            .add_package(name, Some(&uri), Some(name), None);
        self.graph.meta_mut().register_ns_uri(p);
        p
    }

    pub fn class(&mut self, p: PackageId, name: &str, supers: &[ClassId]) -> ClassId {
        let c = self.graph.meta_mut().add_class(p, name, false, false);
        self.graph
            .meta_mut()
            .class_mut(c)
            .supers
            .extend_from_slice(supers);
        c
    }

    pub fn attr(&mut self, c: ClassId, name: &str, dt: &str, many: bool) -> FeatureId {
        let meta = self.graph.meta();
        let ecore = meta
            .package_by_ns_uri("http://www.eclipse.org/emf/2002/Ecore")
            .unwrap();
        let ty = meta.classifier_by_name(ecore, dt).unwrap();
        let f = self
            .graph
            .meta_mut()
            .add_feature(c, name, FeatureKind::Attribute, ty);
        if many {
            self.graph.meta_mut().feature_mut(f).upper = -1;
        }
        f
    }

    pub fn reference(
        &mut self,
        c: ClassId,
        name: &str,
        to: ClassId,
        many: bool,
        containment: bool,
    ) -> FeatureId {
        let f = self.graph.meta_mut().add_feature(
            c,
            name,
            FeatureKind::Reference,
            Classifier::Class(to),
        );
        let fi = self.graph.meta_mut().feature_mut(f);
        if many {
            fi.upper = -1;
        }
        fi.containment = containment;
        f
    }

    /// Finishes the metamodel and copies it into the program (Stdlib classes re-registered).
    pub fn finish_meta(&mut self) {
        self.graph.meta_mut().finish_classes().unwrap();
        let mut meta = self.graph.meta().clone();
        self.p.std_classes = StdClasses::register(&mut meta);
        self.p.meta = meta;
    }

    pub fn class_ty(&mut self, c: ClassId) -> TypeId {
        self.p.types.class(c)
    }

    /// A model type over a package and a model parameter of the transformation.
    pub fn model_param(&mut self, name: &str, pkg: PackageId, dir: Direction) -> ModelParamRef {
        let mt_index = self.p.model_types.len() as u32;
        let ty = self.p.types.intern(Type::ModelType(mt_index));
        self.p.model_types.push(ModelTypeDecl {
            name: format!("MM{mt_index}"),
            module: ModuleId(0),
            packages: vec![pkg],
            strict: false,
            where_: None,
            ty,
        });
        self.p.modules[0].model_types.push(ModelTypeId(mt_index));
        let index = self.p.modules[0].model_params.len() as u32;
        self.p.modules[0].model_params.push(ModelParam {
            name: name.into(),
            direction: dir,
            model_type: ModelTypeId(mt_index),
            ty,
            span: sp(0),
        });
        ModelParamRef {
            module: ModuleId(0),
            index,
        }
    }

    /// Compiles and runs; returns the outcome and the log.
    pub fn run(self) -> (RunOutcome, String) {
        self.run_with(RunOptions::default(), Vec::new())
    }

    pub fn run_with(
        mut self,
        opts: RunOptions,
        args: Vec<Vec<qvto_emf::ObjId>>,
    ) -> (RunOutcome, String) {
        let prog = qvto_bytecode::compile(self.p).expect("compile");
        let mut args = args;
        let n = prog.ir.modules[0].model_params.len();
        while args.len() < n {
            args.push(Vec::new());
        }
        let mut vm = Vm::new(&prog, &mut self.graph, opts);
        let out = vm.run_transformation(args);
        let log = vm.log.clone();
        (out, log)
    }
}

pub fn local(l: LocalId, ty: TypeId) -> Expr {
    e(ExprKind::Local(l), ty)
}

pub fn assign(l: LocalId, v: Expr, ty: TypeId) -> Expr {
    e(
        ExprKind::Assign(Box::new(Assign {
            target: LValue::Local(l),
            target_ty: ty,
            value: v,
            add: false,
        })),
        ty,
    )
}

pub fn var_init(l: LocalId, init: Option<Expr>) -> Expr {
    e(
        ExprKind::VarInit {
            local: l,
            init: init.map(Box::new),
            with_result: false,
        },
        TypeId::OCL_VOID,
    )
}

pub fn to_string(x: Expr) -> Expr {
    std(
        sop(StdOwner::OclAny, "toString", 0),
        Some(x),
        vec![],
        TypeId::STRING,
    )
}

pub fn repr(x: Expr) -> Expr {
    std(
        sop(StdOwner::OclAny, "repr", 0),
        Some(x),
        vec![],
        TypeId::STRING,
    )
}

pub fn lp(
    kind: LoopKind,
    src: Expr,
    it: LocalId,
    body: Option<Expr>,
    cond: Option<Expr>,
    ty: TypeId,
) -> Expr {
    e(
        ExprKind::Loop(Box::new(Loop {
            kind,
            source: src,
            iterators: vec![it],
            acc: None,
            target: None,
            body,
            cond,
            filter_type: None,
        })),
        ty,
    )
}

pub fn coll_lit(k: CollKind, items: Vec<Expr>, ty: TypeId) -> Expr {
    e(
        ExprKind::Coll(k, items.into_iter().map(CollPart::Item).collect()),
        ty,
    )
}

pub fn call(op: OpId, src: Option<Expr>, args: Vec<Expr>, ty: TypeId) -> Expr {
    e(
        ExprKind::Call(Box::new(Call {
            callee: Callee::Op {
                op,
                virtual_: false,
            },
            source: src,
            args,
        })),
        ty,
    )
}

pub fn map(op: OpId, src: Option<Expr>, args: Vec<Expr>, ty: TypeId, strict: bool) -> Expr {
    e(
        ExprKind::Call(Box::new(Call {
            callee: Callee::Mapping {
                op,
                strict,
                virtual_: false,
            },
            source: src,
            args,
        })),
        ty,
    )
}
