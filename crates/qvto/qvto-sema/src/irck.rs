//! Consistency checks of a [`Program`] produced for error-free units: what the bytecode compiler
//! may rely on. Used by the tests over the whole corpus.

use crate::ir::*;
use qvto_types::{Type, TypeId};

/// Checks the invariants of a program compiled without errors; returns the violations.
pub fn check_program(p: &Program) -> Vec<String> {
    let mut c = Checker { p, out: Vec::new() };
    for (i, op) in p.operations.iter().enumerate() {
        c.op(OpId(i as u32), op);
    }
    for g in &p.globals {
        if let Some(b) = &g.init {
            c.body(&format!("global {}", g.name), b);
        }
    }
    for ip in &p.interm_props {
        if let Some(b) = &ip.init {
            c.body(&format!("property {}", ip.name), b);
        }
    }
    for ic in &p.interm_classes {
        for (_, b) in &ic.inits {
            c.body("class init", b);
        }
    }
    for (i, m) in p.modules.iter().enumerate() {
        for &o in &m.operations {
            if o.index() >= p.operations.len()
                || p.operations[o.index()].module != ModuleId(i as u32)
            {
                c.out
                    .push(format!("module {}: foreign operation {o:?}", m.name));
            }
        }
        if let Some(e) = m.entry
            && p.operations[e.index()].module != ModuleId(i as u32)
        {
            c.out
                .push(format!("module {}: entry of another module", m.name));
        }
    }
    c.out
}

struct Checker<'a> {
    p: &'a Program,
    out: Vec<String>,
}

struct Scope<'a> {
    what: String,
    locals: &'a [Local],
}

impl Checker<'_> {
    fn op(&mut self, id: OpId, op: &Operation) {
        let what = format!("op {} ({:?})", op.name, id);
        for p in op.params.iter().chain(op.results.iter()) {
            if p.local.index() >= op.locals.len() {
                self.out
                    .push(format!("{what}: parameter {} has no slot", p.name));
            }
        }
        let s = Scope {
            what,
            locals: &op.locals,
        };
        if let Some(b) = &op.body {
            for e in b {
                self.expr(&s, e);
            }
        }
        if let Some(m) = &op.mapping {
            for e in m
                .when
                .iter()
                .chain(&m.where_)
                .chain(&m.init)
                .chain(&m.population)
                .chain(&m.end)
            {
                self.expr(&s, e);
            }
        }
        if op.kind == OpKind::Mapping
            && op.mapping.is_none()
            && !op.is_blackbox
            && !op.is_abstract
            && op.body.is_none()
        {
            // declarations without body are fine
        }
    }

    fn body(&mut self, what: &str, b: &CodeBody) {
        let s = Scope {
            what: what.to_string(),
            locals: &b.locals,
        };
        self.expr(&s, &b.expr);
    }

    fn local(&mut self, s: &Scope<'_>, l: LocalId) {
        if l.index() >= s.locals.len() {
            self.out
                .push(format!("{}: local {l:?} out of range", s.what));
        } else if s.locals[l.index()].ty == TypeId::NULL {
            self.out.push(format!(
                "{}: local {} has no type",
                s.what,
                s.locals[l.index()].name
            ));
        }
    }

    fn expr(&mut self, s: &Scope<'_>, e: &Expr) {
        if e.ty == TypeId::NULL {
            self.out.push(format!(
                "{}: expression at {:?} has no type ({:?})",
                s.what,
                e.span,
                kind_name(&e.kind)
            ));
        }
        match &e.kind {
            ExprKind::Local(l) => self.local(s, *l),
            ExprKind::Let(l, _, _) | ExprKind::Compute(l, _, _) => self.local(s, *l),
            ExprKind::VarInit { local, .. } => self.local(s, *local),
            ExprKind::Loop(lp) => {
                for &i in &lp.iterators {
                    self.local(s, i);
                }
                if let Some((a, _)) = &lp.acc {
                    self.local(s, *a);
                }
                if let Some(t) = lp.target {
                    self.local(s, t);
                }
            }
            ExprKind::Object(o) => {
                self.local(s, o.var);
            }
            ExprKind::Call(c) => {
                if let Callee::Op { op, .. } | Callee::Mapping { op, .. } = c.callee
                    && op.index() >= self.p.operations.len()
                {
                    self.out.push(format!("{}: call of unknown op", s.what));
                }
                if let Callee::Meta(m) = c.callee
                    && m.index() >= self.p.meta_ops.len()
                {
                    self.out.push(format!("{}: unknown metamodel op", s.what));
                }
            }
            ExprKind::Type(t) => {
                if *t == TypeId::NULL {
                    self.out
                        .push(format!("{}: type expression without type", s.what));
                }
            }
            ExprKind::New(n) => {
                if matches!(self.p.types.get(n.ty), Type::Null) {
                    self.out.push(format!("{}: new without type", s.what));
                }
            }
            _ => {}
        }
        for_each_child(e, |c| self.expr(s, c));
    }
}

fn kind_name(k: &ExprKind) -> &'static str {
    match k {
        ExprKind::Call(_) => "call",
        ExprKind::Loop(_) => "loop",
        ExprKind::Local(_) => "local",
        ExprKind::Object(_) => "object",
        ExprKind::Resolve(_) => "resolve",
        ExprKind::New(_) => "new",
        ExprKind::Switch(..) => "switch",
        ExprKind::If(..) => "if",
        ExprKind::Global(_) => "global",
        ExprKind::Prop(..) => "property",
        ExprKind::ModuleProp(..) => "module property",
        ExprKind::Null => "null",
        _ => "other",
    }
}

/// Calls `f` on the direct children of an expression (all sub-expressions the VM evaluates).
pub fn for_each_child(e: &Expr, mut f: impl FnMut(&Expr)) {
    match &e.kind {
        ExprKind::Coll(_, parts) | ExprKind::List(parts) => {
            for p in parts {
                match p {
                    CollPart::Item(x) => f(x),
                    CollPart::Range(a, b) => {
                        f(a);
                        f(b);
                    }
                }
            }
        }
        ExprKind::Dict(parts) => {
            for (k, v) in parts {
                f(k);
                f(v);
            }
        }
        ExprKind::Tuple(parts) => {
            for (_, v) in parts {
                f(v);
            }
        }
        ExprKind::Prop(s, _)
        | ExprKind::TupleProp(s, _)
        | ExprKind::IntermProp(s, _)
        | ExprKind::ModuleProp(s, _) => f(s),
        ExprKind::Call(c) => {
            if let Some(s) = &c.source {
                f(s);
            }
            for a in &c.args {
                f(a);
            }
        }
        ExprKind::Loop(l) => {
            f(&l.source);
            if let Some((_, i)) = &l.acc {
                f(i);
            }
            if let Some(b) = &l.body {
                f(b);
            }
            if let Some(c) = &l.cond {
                f(c);
            }
        }
        ExprKind::Let(_, i, b) => {
            f(i);
            f(b);
        }
        ExprKind::If(c, t, el) => {
            f(c);
            f(t);
            if let Some(x) = el {
                f(x);
            }
        }
        ExprKind::Block(b) => b.iter().for_each(f),
        ExprKind::VarInit { init: Some(i), .. } => f(i),
        ExprKind::Assign(a) => {
            match &a.target {
                LValue::Prop(s, _)
                | LValue::IntermProp(s, _)
                | LValue::TupleProp(s, _)
                | LValue::ModuleProp(s, _) => f(s),
                _ => {}
            }
            f(&a.value);
        }
        ExprKind::While(c, b) => {
            f(c);
            f(b);
        }
        ExprKind::Compute(_, i, b) => {
            if let Some(i) = i {
                f(i);
            }
            f(b);
        }
        ExprKind::Switch(alts, els) => {
            for (c, b) in alts {
                f(c);
                f(b);
            }
            if let Some(x) = els {
                f(x);
            }
        }
        ExprKind::Return(Some(v)) => f(v),
        ExprKind::Log(l) => {
            for x in [&l.message, &l.element, &l.level, &l.when]
                .into_iter()
                .flatten()
            {
                f(x);
            }
        }
        ExprKind::Assert(a) => {
            f(&a.cond);
            if let Some(l) = &a.log {
                for x in [&l.message, &l.element, &l.level, &l.when]
                    .into_iter()
                    .flatten()
                {
                    f(x);
                }
            }
        }
        ExprKind::Raise(_, Some(a)) => f(a),
        ExprKind::Try(t) => {
            f(&t.body);
            for c in &t.catches {
                f(&c.body);
            }
        }
        ExprKind::Object(o) => o.body.iter().for_each(f),
        ExprKind::New(n) => n.args.iter().for_each(f),
        ExprKind::NewTransformation(_, args) => args.iter().for_each(f),
        ExprKind::Resolve(r) => {
            if let Some(s) = &r.source {
                f(s);
            }
            if let Some(c) = &r.cond {
                f(c);
            }
        }
        _ => {}
    }
}
