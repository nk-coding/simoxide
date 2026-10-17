//! OCL bodies of metamodel operations and OCL derivations of features (Eclipse QVTo's
//! `OCLAnnotationSupport`): the `body`/`derive` details of `http://www.eclipse.org/2007/OCL`
//! annotations of dynamic classes. Eclipse parses them with classic OCL in the context of the
//! owning class when they are first evaluated; a text that does not parse is evaluated as
//! `invalid` (the parser exception is logged). Here they are analysed once after the units, in
//! the environment of the main module with `self` (and the parameters) as variables; problems
//! are not reported, a body with errors is `invalid`.

use crate::analyzer::Sema;
use crate::diag::Problems;
use crate::env::VarRef;
use crate::ir::{self, ExprKind, LocalKind};
use qvto_syntax::Span;
use qvto_types::TypeId;
use std::rc::Rc;

impl<'r> Sema<'r> {
    pub(crate) fn analyse_annotation_bodies(&mut self, main: usize) {
        let has_bodies = self.meta_op_bodies.iter().any(Option::is_some);
        if !has_bodies && self.derivation_texts.is_empty() {
            return;
        }
        let Some(&module) = self.units[main].modules.first() else {
            return;
        };
        let (saved_unit, saved_module) = (self.cur_unit, self.cur_module);
        self.cur_unit = main;
        self.cur_module = module;
        let saved_text = self.units[main].source.text.clone();
        let saved_problems = std::mem::take(&mut self.units[main].problems);
        for i in 0..self.meta_op_bodies.len() {
            let Some(text) = self.meta_op_bodies[i].clone() else {
                continue;
            };
            let mo = self.meta_ops[i].clone();
            let names = self.meta_op_names[i].clone();
            let params: Vec<(String, TypeId)> = names.into_iter().zip(mo.params).collect();
            let class_ty = self.ts.types.class(mo.class);
            let (code, locals) = self.annotation_body(module, &text, class_ty, &params);
            self.meta_ops[i].body = Some(ir::MetaBody {
                code,
                params: locals,
            });
        }
        for d in std::mem::take(&mut self.derivation_texts) {
            let owner = self.ts.meta.feature(d.feature).owner;
            let class_ty = self.ts.types.class(owner);
            let (code, _) = self.annotation_body(module, &d.expr, class_ty, &[]);
            self.derivations.push((d.feature, code));
        }
        self.units[main].source.text = saved_text;
        self.units[main].problems = saved_problems;
        self.cur_unit = saved_unit;
        self.cur_module = saved_module;
    }

    /// One body: the analysed expression with its variables, or `invalid`.
    fn annotation_body(
        &mut self,
        module: ir::ModuleId,
        text: &str,
        self_ty: TypeId,
        params: &[(String, TypeId)],
    ) -> (ir::CodeBody, Vec<ir::LocalId>) {
        let main = self.cur_unit;
        self.units[main].source.text = Rc::from(text);
        self.units[main].problems = Problems::default();
        let span = Span::new(0, text.len() as u32);
        self.begin_code(module, None);
        self.fun.ocl_annotation = true;
        let selfl = self
            .fun
            .new_local("self", self_ty, LocalKind::SelfVar, span);
        self.fun.push_scope();
        self.add_var("self", VarRef::Local(selfl), false);
        let mut locals = Vec::new();
        for (n, t) in params {
            let l = self.fun.new_local(n, *t, LocalKind::Param, span);
            self.add_var(n, VarRef::Local(l), true);
            locals.push(l);
        }
        let (expr, diags) = qvto_syntax::parse_expression(text);
        let analysed = if diags.iter().any(|d| d.is_error()) {
            None
        } else {
            self.visit_ocl_expression(&expr)
        };
        let failed = self.units[main].problems.has_errors();
        let vars = std::mem::take(&mut self.fun.locals);
        self.end_code();
        let expr = match analysed {
            Some(x) if !failed && !x.err => x.e,
            _ => ir::Expr {
                kind: ExprKind::Invalid,
                ty: TypeId::OCL_INVALID,
                span,
            },
        };
        (
            ir::CodeBody {
                source: ir::SourceId(0),
                locals: vars,
                self_local: Some(selfl),
                expr,
            },
            locals,
        )
    }
}
