//! Pretty-printer (unparser): turns an AST back into QVTo source that parses to the same AST
//! (compared with [`crate::dump`]). Comments and the original layout are not preserved; string
//! literals are re-quoted with single quotes and escapes, integer literals printed as values.

use crate::ast::*;
use crate::escape::quote;
use crate::generated::TokenKind;

/// Prints a unit.
pub fn unit(u: &Unit) -> String {
    let mut p = Printer {
        out: String::new(),
        indent: 0,
    };
    for e in &u.elements {
        p.element(e);
    }
    p.out
}

/// Prints an expression.
pub fn expr(e: &Expr) -> String {
    let mut p = Printer {
        out: String::new(),
        indent: 0,
    };
    p.expr(e);
    p.out
}

struct Printer {
    out: String,
    indent: usize,
}

fn kw(k: TokenKind) -> &'static str {
    k.spelling().unwrap_or(k.lpg_name())
}

impl Printer {
    fn w(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn nl(&mut self) {
        self.out.push('\n');
        for _ in 0..self.indent {
            self.out.push('\t');
        }
    }

    fn name(&mut self, n: &Name) {
        match n.kind {
            NameKind::Ident => self.w(&n.text),
            NameKind::Keyword(k) => self.w(kw(k)),
            NameKind::Quoted => {
                self.w("_");
                let q = quote(&n.text);
                self.w(&q);
            }
        }
    }

    fn sep_names(&mut self, ns: &[Name], sep: &str) {
        for (i, n) in ns.iter().enumerate() {
            if i > 0 {
                self.w(sep);
            }
            self.name(n);
        }
    }

    fn string(&mut self, s: &StringLit) {
        let q = quote(&s.value);
        self.w(&q);
    }

    fn scoped(&mut self, s: &ScopedName) {
        if let Some(t) = &s.context_type {
            self.ty(t);
            self.w("::");
        }
        self.sep_names(&s.names, "::");
    }

    fn ty(&mut self, t: &TypeExpr) {
        match &t.kind {
            TypeKind::Primitive(k) => self.w(kw(*k)),
            TypeKind::Path(p) => self.sep_names(&p.segments, "::"),
            TypeKind::Collection(k, e) => {
                self.w(kw(*k));
                self.w("(");
                self.ty(e);
                self.w(")");
            }
            TypeKind::Tuple(parts) => {
                self.w("Tuple(");
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.var(p);
                }
                self.w(")");
            }
            TypeKind::List(e) => {
                self.w("List(");
                self.ty(e);
                self.w(")");
            }
            TypeKind::Dict(k, v) => {
                self.w("Dict(");
                self.ty(k);
                self.w(", ");
                self.ty(v);
                self.w(")");
            }
            TypeKind::Error => self.w("OclInvalid"),
        }
    }

    fn type_spec(&mut self, t: &TypeSpec) {
        self.ty(&t.ty);
        if let Some(e) = &t.extent {
            self.w(" @ ");
            self.name(e);
        }
    }

    fn var(&mut self, v: &VarDecl) {
        self.name(&v.name);
        if let Some(t) = &v.ty {
            self.w(" : ");
            self.ty(t);
        }
        if let Some((op, e)) = &v.init {
            self.w(" ");
            self.w(kw(*op));
            self.w(" ");
            self.expr(e);
        }
    }

    fn block(&mut self, b: &Block) {
        self.w("{");
        self.indent += 1;
        for s in &b.stmts {
            self.nl();
            self.expr(s);
            self.w(";");
        }
        self.indent -= 1;
        self.nl();
        self.w("}");
    }

    fn params(&mut self, ps: &[Param]) {
        for (i, p) in ps.iter().enumerate() {
            if i > 0 {
                self.w(", ");
            }
            if let Some(d) = p.direction {
                self.w(kw(d));
                self.w(" ");
            }
            if let Some(n) = &p.name {
                self.name(n);
                self.w(" : ");
            }
            self.type_spec(&p.ty);
        }
    }

    fn qualifiers(&mut self, qs: &[TokenKind]) {
        for q in qs {
            self.w(kw(*q));
            self.w(" ");
        }
    }

    fn signature(&mut self, params: &[Param], results: &[Param]) {
        self.w("(");
        self.params(params);
        self.w(")");
        if !results.is_empty() {
            self.w(" : ");
            self.params(results);
        }
    }

    fn element(&mut self, e: &Element) {
        match e {
            Element::Import(i) => {
                self.w("import ");
                if i.library {
                    self.w("library ");
                }
                self.sep_names(&i.unit, ".");
                self.w(";");
            }
            Element::Module(m) => {
                self.qualifiers(&m.qualifiers);
                self.w(kw(m.kind));
                self.w(" ");
                self.sep_names(&m.name, ".");
                if let Some(ps) = &m.params {
                    self.w("(");
                    self.params(ps);
                    self.w(")");
                }
                for u in &m.usages {
                    self.w(" ");
                    self.w(kw(u.kind));
                    self.w(" ");
                    if let Some(k) = u.module_kind {
                        self.w(kw(k));
                        self.w(" ");
                    }
                    for (i, r) in u.refs.iter().enumerate() {
                        if i > 0 {
                            self.w(", ");
                        }
                        self.module_ref(r);
                    }
                }
                if let Some(r) = &m.refines {
                    self.w(" refines ");
                    self.module_ref(r);
                }
                match &m.body {
                    None => self.w(";"),
                    Some(els) => {
                        self.w(" {");
                        self.indent += 1;
                        for e in els {
                            self.nl();
                            self.element(e);
                        }
                        self.indent -= 1;
                        self.nl();
                        self.w("}");
                    }
                }
            }
            Element::ModelType(m) => {
                self.w("modeltype ");
                self.name(&m.name);
                if let Some(c) = &m.compliance {
                    self.w(" ");
                    self.string(c);
                }
                self.w(" uses ");
                for (i, p) in m.packages.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    if let Some(path) = &p.path {
                        self.sep_names(&path.segments, "::");
                        if let Some(u) = &p.uri {
                            self.w("(");
                            self.string(u);
                            self.w(")");
                        }
                    } else if let Some(u) = &p.uri {
                        self.string(u);
                    }
                }
                if let Some(w) = &m.where_ {
                    self.w(" where ");
                    self.block(w);
                }
                self.w(";");
            }
            Element::Class(c) => {
                self.w(if c.exception {
                    "exception "
                } else {
                    "intermediate class "
                });
                self.name(&c.name);
                if !c.extends.is_empty() {
                    self.w(" extends ");
                    for (i, t) in c.extends.iter().enumerate() {
                        if i > 0 {
                            self.w(", ");
                        }
                        self.ty(t);
                    }
                }
                self.w(" {");
                self.indent += 1;
                for f in &c.features {
                    self.nl();
                    match f {
                        ClassFeature::Tag(t) => self.tag(t),
                        ClassFeature::Property(p) => {
                            if !p.stereotypes.is_empty() {
                                self.w("<<");
                                self.sep_names(&p.stereotypes, ", ");
                                self.w(">> ");
                            }
                            self.qualifiers(&p.keys);
                            self.name(&p.name);
                            self.w(" : ");
                            self.ty(&p.ty);
                            if let Some(m) = &p.multiplicity {
                                self.mult(m);
                            }
                            if p.ordered {
                                self.w(" ordered");
                            }
                            if let Some(o) = &p.opposite {
                                self.w(" opposites ");
                                if o.navigable {
                                    self.w("~");
                                }
                                self.name(&o.name);
                                if let Some(m) = &o.multiplicity {
                                    self.mult(m);
                                }
                            }
                            if let Some(i) = &p.init {
                                self.w(" = ");
                                self.expr(i);
                            }
                        }
                    }
                    self.w(";");
                }
                self.indent -= 1;
                self.nl();
                self.w("}");
            }
            Element::Property(p) => {
                self.w(match p.kind {
                    PropertyKind::Configuration => "configuration property ",
                    PropertyKind::Plain => "property ",
                    PropertyKind::Intermediate => "intermediate property ",
                });
                self.scoped(&p.name);
                if let Some(t) = &p.ty {
                    self.w(" : ");
                    self.ty(t);
                }
                if let Some(i) = &p.init {
                    self.w(" = ");
                    self.expr(i);
                }
                self.w(";");
            }
            Element::Helper(h) => {
                self.qualifiers(&h.qualifiers);
                self.w(kw(h.kind));
                self.w(" ");
                self.scoped(&h.name);
                self.signature(&h.params, &h.results);
                match &h.body {
                    OpBody::None => self.w(";"),
                    OpBody::Expr(e) => {
                        self.w(" = ");
                        self.expr(e);
                        self.w(";");
                    }
                    OpBody::Block(b) => {
                        self.w(" ");
                        self.block(b);
                    }
                }
            }
            Element::Constructor(c) => {
                self.qualifiers(&c.qualifiers);
                self.w("constructor ");
                self.scoped(&c.name);
                self.signature(&c.params, &[]);
                self.opt_body(&c.body);
            }
            Element::Entry(x) => {
                self.w("main");
                self.signature(&x.params, &[]);
                self.opt_body(&x.body);
            }
            Element::Mapping(m) => {
                self.qualifiers(&m.qualifiers);
                self.w("mapping ");
                if let Some(d) = m.direction {
                    self.w(kw(d));
                    self.w(" ");
                }
                self.scoped(&m.name);
                self.signature(&m.params, &m.results);
                for x in &m.extensions {
                    self.w(" ");
                    self.w(kw(x.kind));
                    self.w(" ");
                    for (i, n) in x.names.iter().enumerate() {
                        if i > 0 {
                            self.w(", ");
                        }
                        self.scoped(n);
                    }
                }
                if let Some(b) = &m.when {
                    self.w(" when ");
                    self.block(b);
                }
                if let Some(b) = &m.where_ {
                    self.w(" where ");
                    self.block(b);
                }
                match &m.body {
                    None => self.w(";"),
                    Some(b) => {
                        self.w(" {");
                        self.indent += 1;
                        if let Some(i) = &b.init {
                            self.nl();
                            self.w("init ");
                            self.block(i);
                        }
                        if let Some(p) = &b.population {
                            if p.explicit {
                                self.nl();
                                self.w("population ");
                                self.block(&p.body);
                            } else {
                                for s in &p.body.stmts {
                                    self.nl();
                                    self.expr(s);
                                    self.w(";");
                                }
                            }
                        }
                        if let Some(e) = &b.end {
                            self.nl();
                            self.w("end ");
                            self.block(e);
                        }
                        self.indent -= 1;
                        self.nl();
                        self.w("}");
                    }
                }
            }
            Element::Tag(t) => {
                self.tag(t);
                self.w(";");
            }
            Element::Rename(r) => {
                self.w("rename ");
                self.ty(&r.ty);
                self.w(".");
                self.name(&r.property);
                self.w(" = ");
                self.string(&r.new_name);
                self.w(";");
            }
            Element::Error(_) => {}
        }
        self.nl();
    }

    fn opt_body(&mut self, b: &Option<Block>) {
        match b {
            None => self.w(";"),
            Some(b) => {
                self.w(" ");
                self.block(b);
            }
        }
    }

    fn module_ref(&mut self, r: &ModuleRef) {
        self.sep_names(&r.path.segments, "::");
        if let Some(ps) = &r.params {
            self.w("(");
            self.params(ps);
            self.w(")");
        }
    }

    fn mult(&mut self, m: &Multiplicity) {
        let b = |b: MultBound| match b {
            MultBound::Int(i) => i.to_string(),
            MultBound::Many => "*".to_string(),
        };
        self.w("[");
        self.w(&b(m.lower));
        if let Some(u) = m.upper {
            self.w("..");
            self.w(&b(u));
        }
        self.w("]");
    }

    fn tag(&mut self, t: &Tag) {
        self.w("tag ");
        self.string(&t.id);
        self.w(" ");
        self.scoped(&t.element);
        if let Some(v) = &t.value {
            self.w(" = ");
            self.expr(v);
        }
    }

    fn source(&mut self, s: &Option<(Expr, NavOp)>) {
        if let Some((e, op)) = s {
            self.operand(e);
            self.w(match op {
                NavOp::Dot => ".",
                NavOp::Arrow => "->",
            });
        }
    }

    /// Prints a postfix source; non-primaries need parentheses, which the AST does not have
    /// (they come from `Paren` nodes), so print as is.
    fn operand(&mut self, e: &Expr) {
        self.expr(e);
    }

    fn args(&mut self, args: &[Expr]) {
        self.w("(");
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.w(", ");
            }
            self.expr(a);
        }
        self.w(")");
    }

    fn parts(&mut self, parts: &[CollectionPart]) {
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                self.w(", ");
            }
            match p {
                CollectionPart::Item(e) => self.expr(e),
                CollectionPart::Range(a, b) => {
                    self.expr(a);
                    self.w(" .. ");
                    self.expr(b);
                }
            }
        }
    }

    fn switch_body(&mut self, b: &SwitchBody) {
        self.w("{");
        self.indent += 1;
        for a in &b.alts {
            self.nl();
            if a.deprecated {
                self.w("(");
                self.expr(&a.cond);
                self.w(") ? ");
                self.expr(&a.body);
                self.w(";");
            } else {
                self.w("case (");
                self.expr(&a.cond);
                self.w(") ");
                self.stmt_body(&a.body);
            }
        }
        if let Some(e) = &b.els {
            self.nl();
            if e.deprecated {
                self.w("else ? ");
                self.expr(&e.body);
                self.w(";");
            } else {
                self.w("else ");
                self.stmt_body(&e.body);
            }
        }
        self.indent -= 1;
        self.nl();
        self.w("}");
    }

    /// `expression_statement`
    fn stmt_body(&mut self, e: &Expr) {
        if let ExprKind::Block(b) = &e.kind {
            self.block(b);
        } else {
            self.expr(e);
            self.w(";");
        }
    }

    fn object_decl(&mut self, d: &ObjectDecl) {
        if let Some(v) = &d.var {
            self.name(v);
            self.w(" : ");
        }
        if let Some(t) = &d.ty {
            self.type_spec(t);
            self.w(" ");
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Int(v) => self.w(&v.to_string()),
            ExprKind::Real(s) => self.w(s),
            ExprKind::Str(s) => self.string(s),
            ExprKind::Bool(b) => self.w(if *b { "true" } else { "false" }),
            ExprKind::Null => self.w("null"),
            ExprKind::Invalid => self.w("invalid"),
            ExprKind::Unlimited => self.w("*"),
            ExprKind::SelfExp => self.w("self"),
            ExprKind::Name(n) => self.name(n),
            ExprKind::Path(p) => self.sep_names(&p.segments, "::"),
            ExprKind::TypeLit(t) => self.ty(t),
            ExprKind::CollectionLit(k, parts) => {
                self.w(kw(*k));
                self.w("{");
                self.parts(parts);
                self.w("}");
            }
            ExprKind::ListLit(parts) => {
                self.w("List{");
                self.parts(parts);
                self.w("}");
            }
            ExprKind::DictLit(parts) => {
                self.w("Dict{");
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.expr(&p.key);
                    self.w(" = ");
                    self.expr(&p.value);
                }
                self.w("}");
            }
            ExprKind::TupleLit(parts) => {
                self.w("Tuple{");
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.var(p);
                }
                self.w("}");
            }
            ExprKind::Paren(inner) => {
                self.w("(");
                self.expr(inner);
                self.w(")");
            }
            ExprKind::Unary(op, x) => {
                self.w(kw(*op));
                // `--` would start a comment.
                if *op != TokenKind::Minus || matches!(x.kind, ExprKind::Unary(TokenKind::Minus, _))
                {
                    self.w(" ");
                }
                self.expr(x);
            }
            ExprKind::Binary(op, a, b) => {
                self.expr(a);
                self.w(" ");
                self.w(kw(*op));
                self.w(" ");
                self.expr(b);
            }
            ExprKind::Let(vars, body) => {
                self.w("let ");
                for (i, v) in vars.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.var(v);
                }
                self.w(" in ");
                self.expr(body);
            }
            ExprKind::If(i) => {
                if i.c_style {
                    self.w("if (");
                    self.expr(&i.cond);
                    self.w(") ");
                    self.expr(&i.then);
                    for (c, b) in &i.elifs {
                        self.w(" elif (");
                        self.expr(c);
                        self.w(") ");
                        self.expr(b);
                    }
                    if let Some(e) = &i.els {
                        self.w(" else ");
                        self.expr(e);
                    }
                    if i.endif {
                        self.w(" endif");
                    }
                } else {
                    self.w("if ");
                    self.expr(&i.cond);
                    self.w(" then ");
                    self.expr(&i.then);
                    for (c, b) in &i.elifs {
                        self.w(" elif ");
                        self.expr(c);
                        self.w(" then ");
                        self.expr(b);
                    }
                    if let Some(e) = &i.els {
                        self.w(" else ");
                        self.expr(e);
                    }
                    self.w(" endif");
                }
            }
            ExprKind::Call(c) => {
                self.source(&c.source);
                for q in &c.qualifier {
                    self.name(q);
                    self.w("::");
                }
                self.name(&c.name);
                self.args(&c.args);
            }
            ExprKind::Nav(n) => {
                self.operand(&n.source);
                self.w(match n.op {
                    NavOp::Dot => ".",
                    NavOp::Arrow => "->",
                });
                for q in &n.qualifier {
                    self.name(q);
                    self.w("::");
                }
                self.name(&n.name);
            }
            ExprKind::Iterate(it) => {
                self.operand(&it.source);
                self.w("->");
                self.name(&it.name);
                self.w("(");
                for (i, v) in it.vars.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.var(v);
                }
                if let Some(a) = &it.acc {
                    if !it.vars.is_empty() {
                        self.w("; ");
                    }
                    self.var(a);
                }
                self.w(" | ");
                self.expr(&it.body);
                self.w(")");
            }
            ExprKind::XIterate(x) => {
                self.operand(&x.source);
                self.w("->");
                self.w(kw(x.kind));
                self.w("(");
                for (i, v) in x.vars.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.var(v);
                }
                if let Some(t) = &x.target {
                    self.w("; ");
                    self.var(t);
                }
                if !x.vars.is_empty() || x.target.is_some() {
                    self.w(" | ");
                }
                self.expr(&x.body);
                self.w(")");
            }
            ExprKind::XSelect(x) => {
                self.operand(&x.source);
                if x.one {
                    self.w("!");
                }
                self.w("[");
                if let Some(v) = &x.var {
                    self.name(v);
                    self.w(" | ");
                }
                self.expr(&x.cond);
                self.w("]");
            }
            ExprKind::ForEach(f) => {
                self.operand(&f.source);
                self.w("->");
                self.w(kw(f.kind));
                self.w("(");
                self.sep_names(&f.vars, ", ");
                if let Some(c) = &f.cond {
                    self.w(" | ");
                    self.expr(c);
                }
                self.w(") ");
                self.block(&f.body);
            }
            ExprKind::ObjectIter(o) => {
                self.operand(&o.source);
                self.w("->object(");
                self.var(&o.var);
                self.w(") ");
                self.object_decl(&o.decl);
                self.block(&o.body);
            }
            ExprKind::SwitchIter(s) => {
                self.operand(&s.source);
                self.w("->switch(");
                self.var(&s.var);
                self.w(") ");
                self.switch_body(&s.body);
            }
            ExprKind::Assign(a) => {
                self.expr(&a.lhs);
                self.w(if a.add { " += " } else { " := " });
                self.expr(&a.rhs);
            }
            ExprKind::Var(v) => {
                self.w("var ");
                if v.parens {
                    self.w("(");
                }
                for (i, d) in v.decls.iter().enumerate() {
                    if i > 0 {
                        self.w(", ");
                    }
                    self.var(d);
                }
                if v.parens {
                    self.w(")");
                }
            }
            ExprKind::Return(v) => {
                self.w("return");
                if let Some(v) = v {
                    self.w(" ");
                    self.expr(v);
                }
            }
            ExprKind::Break => self.w("break"),
            ExprKind::Continue => self.w("continue"),
            ExprKind::While(w) => {
                self.w("while (");
                match &w.init {
                    Some(WhileInit::Decl(d)) => {
                        self.var(d);
                        self.w("; ");
                    }
                    Some(WhileInit::Assign(a)) => {
                        self.expr(a);
                        self.w("; ");
                    }
                    None => {}
                }
                self.expr(&w.cond);
                self.w(") ");
                self.block(&w.body);
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::Do(b) => {
                self.w("do ");
                self.block(b);
            }
            ExprKind::Switch(s) => {
                self.w("switch ");
                self.switch_body(s);
            }
            ExprKind::Log(l) => {
                self.w("log");
                self.args(&l.args);
                if let Some(w) = &l.when {
                    self.w(" when ");
                    self.expr(w);
                }
            }
            ExprKind::Assert(a) => {
                self.w("assert ");
                if let Some(s) = &a.severity {
                    self.name(s);
                    self.w(" ");
                }
                self.w("(");
                self.expr(&a.cond);
                self.w(")");
                if let Some(l) = &a.log {
                    self.w(" with ");
                    self.expr(l);
                }
            }
            ExprKind::Compute(c) => {
                self.w("compute (");
                self.var(&c.decl);
                self.w(") ");
                self.block(&c.body);
            }
            ExprKind::New(n) => {
                self.w("new ");
                self.type_spec(&n.ty);
                self.args(&n.args);
            }
            ExprKind::Object(o) => {
                self.w("object ");
                self.object_decl(&o.decl);
                self.block(&o.body);
            }
            ExprKind::MapCall(m) => {
                self.source(&m.source);
                self.w(if m.xmap { "xmap " } else { "map " });
                self.sep_names(&m.name.segments, "::");
                self.args(&m.args);
            }
            ExprKind::Resolve(r) => {
                self.source(&r.source);
                if r.late {
                    self.w("late ");
                }
                self.w(kw(r.op));
                self.w("(");
                if let Some(m) = &r.mapping {
                    self.scoped(m);
                    if r.target.is_some() {
                        self.w(", ");
                    }
                }
                if let Some(t) = &r.target {
                    if let Some(v) = &t.var {
                        self.name(v);
                        self.w(" : ");
                    }
                    self.ty(&t.ty);
                    if let Some(c) = &t.cond {
                        self.w(" | ");
                        self.expr(c);
                    }
                }
                self.w(")");
            }
            ExprKind::Raise(r) => {
                self.w("raise ");
                self.expr(&r.exception);
                match &r.arg {
                    None => {}
                    Some(None) => self.w("()"),
                    Some(Some(a)) => {
                        self.w("(");
                        self.expr(a);
                        self.w(")");
                    }
                }
            }
            ExprKind::Try(t) => {
                self.w("try ");
                self.block(&t.body);
                for c in &t.catches {
                    self.w(" except (");
                    if let Some(v) = &c.var {
                        self.name(v);
                        self.w(" : ");
                    }
                    for (i, ty) in c.types.iter().enumerate() {
                        if i > 0 {
                            self.w(", ");
                        }
                        self.ty(ty);
                    }
                    self.w(") ");
                    self.block(&c.body);
                }
            }
            ExprKind::Error => self.w("invalid"),
        }
    }
}
