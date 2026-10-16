//! Span-free AST dump: an S-expression that is equal for two trees exactly when they are equal
//! up to spans and literal spellings. Used by the round-trip tests (parse → print → parse) and
//! for debugging.

use crate::ast::*;
use crate::generated::TokenKind;
use std::fmt::Write;

/// Dumps a unit.
pub fn unit(u: &Unit) -> String {
    let mut d = Dumper {
        out: String::new(),
        indent: 0,
    };
    d.open("unit");
    for e in &u.elements {
        d.element(e);
    }
    d.close();
    d.out
}

/// Dumps an expression.
pub fn expr(e: &Expr) -> String {
    let mut d = Dumper {
        out: String::new(),
        indent: 0,
    };
    d.expr(e);
    d.out
}

struct Dumper {
    out: String,
    indent: usize,
}

fn kw(k: TokenKind) -> &'static str {
    k.spelling().unwrap_or(k.lpg_name())
}

impl Dumper {
    fn nl(&mut self) {
        self.out.push('\n');
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
    }

    fn open(&mut self, tag: &str) {
        if !self.out.is_empty() {
            self.nl();
        }
        self.out.push('(');
        self.out.push_str(tag);
        self.indent += 1;
    }

    fn close(&mut self) {
        self.indent -= 1;
        self.out.push(')');
    }

    fn atom(&mut self, s: &str) {
        self.out.push(' ');
        self.out.push_str(s);
    }

    fn str_atom(&mut self, s: &str) {
        let _ = write!(self.out, " {s:?}");
    }

    fn leaf(&mut self, tag: &str, s: &str) {
        self.open(tag);
        self.atom(s);
        self.close();
    }

    fn name(&mut self, n: &Name) {
        match n.kind {
            NameKind::Ident => self.str_atom(&n.text),
            NameKind::Quoted => {
                let _ = write!(self.out, " _{:?}", n.text);
            }
            NameKind::Keyword(k) => {
                let _ = write!(self.out, " #{}", kw(k));
            }
        }
    }

    fn names(&mut self, tag: &str, ns: &[Name]) {
        self.open(tag);
        for n in ns {
            self.name(n);
        }
        self.close();
    }

    fn opt_name(&mut self, n: &Option<Name>) {
        match n {
            Some(n) => self.name(n),
            None => self.atom("-"),
        }
    }

    fn scoped(&mut self, s: &ScopedName) {
        self.open("scoped");
        if let Some(t) = &s.context_type {
            self.ty(t);
        }
        for n in &s.names {
            self.name(n);
        }
        self.close();
    }

    fn ty(&mut self, t: &TypeExpr) {
        match &t.kind {
            TypeKind::Primitive(k) => self.leaf("type", kw(*k)),
            TypeKind::Path(p) => self.names("type-path", &p.segments),
            TypeKind::Collection(k, e) => {
                self.open(kw(*k));
                self.ty(e);
                self.close();
            }
            TypeKind::Tuple(parts) => {
                self.open("Tuple");
                for p in parts {
                    self.var(p);
                }
                self.close();
            }
            TypeKind::List(e) => {
                self.open("List");
                self.ty(e);
                self.close();
            }
            TypeKind::Dict(k, v) => {
                self.open("Dict");
                self.ty(k);
                self.ty(v);
                self.close();
            }
            TypeKind::Error => self.leaf("type", "error"),
        }
    }

    fn type_spec(&mut self, t: &TypeSpec) {
        self.open("typespec");
        self.ty(&t.ty);
        if let Some(e) = &t.extent {
            self.name(e);
        }
        self.close();
    }

    fn var(&mut self, v: &VarDecl) {
        self.open("var");
        self.name(&v.name);
        if let Some(t) = &v.ty {
            self.ty(t);
        }
        if let Some((op, e)) = &v.init {
            self.atom(kw(*op));
            self.expr(e);
        }
        self.close();
    }

    fn block(&mut self, tag: &str, b: &Block) {
        self.open(tag);
        for s in &b.stmts {
            self.expr(s);
        }
        self.close();
    }

    fn opt_block(&mut self, tag: &str, b: &Option<Block>) {
        if let Some(b) = b {
            self.block(tag, b);
        }
    }

    fn string(&mut self, s: &StringLit) {
        self.open("str");
        self.str_atom(&s.value);
        self.close();
    }

    fn params(&mut self, tag: &str, ps: &[Param]) {
        self.open(tag);
        for p in ps {
            self.open("param");
            if let Some(d) = p.direction {
                self.atom(kw(d));
            }
            self.opt_name(&p.name);
            self.type_spec(&p.ty);
            self.close();
        }
        self.close();
    }

    fn kws(&mut self, tag: &str, ks: &[TokenKind]) {
        if ks.is_empty() {
            return;
        }
        self.open(tag);
        for k in ks {
            self.atom(kw(*k));
        }
        self.close();
    }

    fn element(&mut self, e: &Element) {
        match e {
            Element::Import(i) => {
                self.open(if i.library {
                    "import-library"
                } else {
                    "import"
                });
                for n in &i.unit {
                    self.name(n);
                }
                self.close();
            }
            Element::Module(m) => {
                self.open(kw(m.kind));
                self.kws("qualifiers", &m.qualifiers);
                self.names("name", &m.name);
                if let Some(ps) = &m.params {
                    self.params("signature", ps);
                }
                for u in &m.usages {
                    self.open(kw(u.kind));
                    if let Some(k) = u.module_kind {
                        self.atom(kw(k));
                    }
                    for r in &u.refs {
                        self.module_ref(r);
                    }
                    self.close();
                }
                if let Some(r) = &m.refines {
                    self.open("refines");
                    self.module_ref(r);
                    self.close();
                }
                if let Some(body) = &m.body {
                    self.open("body");
                    for e in body {
                        self.element(e);
                    }
                    self.close();
                }
                self.close();
            }
            Element::ModelType(m) => {
                self.open("modeltype");
                self.name(&m.name);
                if let Some(c) = &m.compliance {
                    self.string(c);
                }
                for p in &m.packages {
                    self.open("package");
                    if let Some(path) = &p.path {
                        self.names("path", &path.segments);
                    }
                    if let Some(u) = &p.uri {
                        self.string(u);
                    }
                    self.close();
                }
                self.opt_block("where", &m.where_);
                self.close();
            }
            Element::Class(c) => {
                self.open(if c.exception { "exception" } else { "class" });
                self.name(&c.name);
                if !c.extends.is_empty() {
                    self.open("extends");
                    for t in &c.extends {
                        self.ty(t);
                    }
                    self.close();
                }
                for f in &c.features {
                    match f {
                        ClassFeature::Tag(t) => self.tag(t),
                        ClassFeature::Property(p) => {
                            self.open("feature");
                            if !p.stereotypes.is_empty() {
                                self.names("stereotypes", &p.stereotypes);
                            }
                            self.kws("keys", &p.keys);
                            self.name(&p.name);
                            self.ty(&p.ty);
                            if let Some(m) = &p.multiplicity {
                                self.mult(m);
                            }
                            if p.ordered {
                                self.atom("ordered");
                            }
                            if let Some(o) = &p.opposite {
                                self.open("opposite");
                                if o.navigable {
                                    self.atom("~");
                                }
                                self.name(&o.name);
                                if let Some(m) = &o.multiplicity {
                                    self.mult(m);
                                }
                                self.close();
                            }
                            if let Some(i) = &p.init {
                                self.expr(i);
                            }
                            self.close();
                        }
                    }
                }
                self.close();
            }
            Element::Property(p) => {
                self.open(match p.kind {
                    PropertyKind::Configuration => "configuration-property",
                    PropertyKind::Plain => "property",
                    PropertyKind::Intermediate => "intermediate-property",
                });
                self.scoped(&p.name);
                if let Some(t) = &p.ty {
                    self.ty(t);
                }
                if let Some(i) = &p.init {
                    self.expr(i);
                }
                self.close();
            }
            Element::Helper(h) => {
                self.open(kw(h.kind));
                self.kws("qualifiers", &h.qualifiers);
                self.scoped(&h.name);
                self.params("params", &h.params);
                self.params("results", &h.results);
                match &h.body {
                    OpBody::None => {}
                    OpBody::Expr(e) => {
                        self.open("=");
                        self.expr(e);
                        self.close();
                    }
                    OpBody::Block(b) => self.block("body", b),
                }
                self.close();
            }
            Element::Constructor(c) => {
                self.open("constructor");
                self.kws("qualifiers", &c.qualifiers);
                self.scoped(&c.name);
                self.params("params", &c.params);
                self.opt_block("body", &c.body);
                self.close();
            }
            Element::Entry(x) => {
                self.open("main");
                self.params("params", &x.params);
                self.opt_block("body", &x.body);
                self.close();
            }
            Element::Mapping(m) => {
                self.open("mapping");
                self.kws("qualifiers", &m.qualifiers);
                if let Some(d) = m.direction {
                    self.atom(kw(d));
                }
                self.scoped(&m.name);
                self.params("params", &m.params);
                self.params("results", &m.results);
                for x in &m.extensions {
                    self.open(kw(x.kind));
                    for n in &x.names {
                        self.scoped(n);
                    }
                    self.close();
                }
                self.opt_block("when", &m.when);
                self.opt_block("where", &m.where_);
                if let Some(b) = &m.body {
                    self.open("body");
                    self.opt_block("init", &b.init);
                    if let Some(p) = &b.population {
                        self.block(
                            if p.explicit {
                                "population"
                            } else {
                                "implicit-population"
                            },
                            &p.body,
                        );
                    }
                    self.opt_block("end", &b.end);
                    self.close();
                }
                self.close();
            }
            Element::Tag(t) => self.tag(t),
            Element::Rename(r) => {
                self.open("rename");
                self.ty(&r.ty);
                self.name(&r.property);
                self.string(&r.new_name);
                self.close();
            }
            Element::Error(_) => self.leaf("error", "element"),
        }
    }

    fn module_ref(&mut self, r: &ModuleRef) {
        self.open("ref");
        self.names("path", &r.path.segments);
        if let Some(ps) = &r.params {
            self.params("signature", ps);
        }
        self.close();
    }

    fn mult(&mut self, m: &Multiplicity) {
        let b = |b: MultBound| match b {
            MultBound::Int(i) => i.to_string(),
            MultBound::Many => "*".to_string(),
        };
        self.open("mult");
        self.atom(&b(m.lower));
        if let Some(u) = m.upper {
            self.atom(&b(u));
        }
        self.close();
    }

    fn tag(&mut self, t: &Tag) {
        self.open("tag");
        self.string(&t.id);
        self.scoped(&t.element);
        if let Some(v) = &t.value {
            self.expr(v);
        }
        self.close();
    }

    fn source(&mut self, s: &Option<(Expr, NavOp)>) {
        if let Some((e, op)) = s {
            self.open(match op {
                NavOp::Dot => "src.",
                NavOp::Arrow => "src->",
            });
            self.expr(e);
            self.close();
        }
    }

    fn switch_body(&mut self, b: &SwitchBody) {
        for a in &b.alts {
            self.open(if a.deprecated { "alt?" } else { "case" });
            self.expr(&a.cond);
            self.expr(&a.body);
            self.close();
        }
        if let Some(e) = &b.els {
            self.open(if e.deprecated { "else?" } else { "else" });
            self.expr(&e.body);
            self.close();
        }
    }

    fn parts(&mut self, parts: &[CollectionPart]) {
        for p in parts {
            match p {
                CollectionPart::Item(e) => self.expr(e),
                CollectionPart::Range(a, b) => {
                    self.open("..");
                    self.expr(a);
                    self.expr(b);
                    self.close();
                }
            }
        }
    }

    fn object_decl(&mut self, d: &ObjectDecl) {
        self.open("decl");
        self.opt_name(&d.var);
        if let Some(t) = &d.ty {
            self.type_spec(t);
        }
        self.close();
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Int(v) => self.leaf("int", &v.to_string()),
            ExprKind::Real(s) => self.leaf("real", s),
            ExprKind::Str(s) => self.string(s),
            ExprKind::Bool(b) => self.leaf("bool", if *b { "true" } else { "false" }),
            ExprKind::Null => self.leaf("lit", "null"),
            ExprKind::Invalid => self.leaf("lit", "invalid"),
            ExprKind::Unlimited => self.leaf("lit", "*"),
            ExprKind::SelfExp => self.leaf("lit", "self"),
            ExprKind::Name(n) => {
                self.open("name");
                self.name(n);
                self.close();
            }
            ExprKind::Path(p) => self.names("path", &p.segments),
            ExprKind::TypeLit(t) => {
                self.open("type-lit");
                self.ty(t);
                self.close();
            }
            ExprKind::CollectionLit(k, parts) => {
                self.open(&format!("{}{{}}", kw(*k)));
                self.parts(parts);
                self.close();
            }
            ExprKind::ListLit(parts) => {
                self.open("List{}");
                self.parts(parts);
                self.close();
            }
            ExprKind::DictLit(parts) => {
                self.open("Dict{}");
                for p in parts {
                    self.open("=");
                    self.expr(&p.key);
                    self.expr(&p.value);
                    self.close();
                }
                self.close();
            }
            ExprKind::TupleLit(parts) => {
                self.open("Tuple{}");
                for p in parts {
                    self.var(p);
                }
                self.close();
            }
            ExprKind::Paren(inner) => {
                self.open("paren");
                self.expr(inner);
                self.close();
            }
            ExprKind::Unary(op, x) => {
                self.open(&format!("unary {}", kw(*op)));
                self.expr(x);
                self.close();
            }
            ExprKind::Binary(op, a, b) => {
                self.open(kw(*op));
                self.expr(a);
                self.expr(b);
                self.close();
            }
            ExprKind::Let(vars, body) => {
                self.open("let");
                for v in vars {
                    self.var(v);
                }
                self.expr(body);
                self.close();
            }
            ExprKind::If(i) => {
                self.open(match (i.c_style, i.endif) {
                    (false, _) => "if",
                    (true, true) => "if-c-endif",
                    (true, false) => "if-c",
                });
                self.expr(&i.cond);
                self.expr(&i.then);
                for (c, b) in &i.elifs {
                    self.open("elif");
                    self.expr(c);
                    self.expr(b);
                    self.close();
                }
                if let Some(e) = &i.els {
                    self.open("else");
                    self.expr(e);
                    self.close();
                }
                self.close();
            }
            ExprKind::Call(c) => {
                self.open("call");
                self.source(&c.source);
                for q in &c.qualifier {
                    self.name(q);
                    self.atom("::");
                }
                self.name(&c.name);
                for a in &c.args {
                    self.expr(a);
                }
                self.close();
            }
            ExprKind::Nav(n) => {
                self.open(match n.op {
                    NavOp::Dot => "nav.",
                    NavOp::Arrow => "nav->",
                });
                self.expr(&n.source);
                for q in &n.qualifier {
                    self.name(q);
                    self.atom("::");
                }
                self.name(&n.name);
                self.close();
            }
            ExprKind::Iterate(it) => {
                self.open("iterate");
                self.expr(&it.source);
                self.name(&it.name);
                for v in &it.vars {
                    self.var(v);
                }
                if let Some(a) = &it.acc {
                    self.open("acc");
                    self.var(a);
                    self.close();
                }
                self.expr(&it.body);
                self.close();
            }
            ExprKind::XIterate(x) => {
                self.open(kw(x.kind));
                self.expr(&x.source);
                for v in &x.vars {
                    self.var(v);
                }
                if let Some(t) = &x.target {
                    self.open("target");
                    self.var(t);
                    self.close();
                }
                self.expr(&x.body);
                self.close();
            }
            ExprKind::XSelect(x) => {
                self.open(if x.one { "![]" } else { "[]" });
                self.expr(&x.source);
                self.opt_name(&x.var);
                self.expr(&x.cond);
                self.close();
            }
            ExprKind::ForEach(f) => {
                self.open(kw(f.kind));
                self.expr(&f.source);
                for v in &f.vars {
                    self.name(v);
                }
                if let Some(c) = &f.cond {
                    self.open("|");
                    self.expr(c);
                    self.close();
                }
                self.block("body", &f.body);
                self.close();
            }
            ExprKind::ObjectIter(o) => {
                self.open("->object");
                self.expr(&o.source);
                self.var(&o.var);
                self.object_decl(&o.decl);
                self.block("body", &o.body);
                self.close();
            }
            ExprKind::SwitchIter(s) => {
                self.open("->switch");
                self.expr(&s.source);
                self.var(&s.var);
                self.switch_body(&s.body);
                self.close();
            }
            ExprKind::Assign(a) => {
                self.open(if a.add { "+=" } else { ":=" });
                self.expr(&a.lhs);
                self.expr(&a.rhs);
                self.close();
            }
            ExprKind::Var(v) => {
                self.open(if v.parens { "var()" } else { "var-exp" });
                for d in &v.decls {
                    self.var(d);
                }
                self.close();
            }
            ExprKind::Return(v) => {
                self.open("return");
                if let Some(v) = v {
                    self.expr(v);
                }
                self.close();
            }
            ExprKind::Break => self.leaf("lit", "break"),
            ExprKind::Continue => self.leaf("lit", "continue"),
            ExprKind::While(w) => {
                self.open("while");
                match &w.init {
                    Some(WhileInit::Decl(d)) => self.var(d),
                    Some(WhileInit::Assign(a)) => self.expr(a),
                    None => {}
                }
                self.expr(&w.cond);
                self.block("body", &w.body);
                self.close();
            }
            ExprKind::Block(b) => self.block("block", b),
            ExprKind::Do(b) => self.block("do", b),
            ExprKind::Switch(s) => {
                self.open("switch");
                self.switch_body(s);
                self.close();
            }
            ExprKind::Log(l) => {
                self.open("log");
                for a in &l.args {
                    self.expr(a);
                }
                if let Some(w) = &l.when {
                    self.open("when");
                    self.expr(w);
                    self.close();
                }
                self.close();
            }
            ExprKind::Assert(a) => {
                self.open("assert");
                self.opt_name(&a.severity);
                self.expr(&a.cond);
                if let Some(l) = &a.log {
                    self.expr(l);
                }
                self.close();
            }
            ExprKind::Compute(c) => {
                self.open("compute");
                self.var(&c.decl);
                self.block("body", &c.body);
                self.close();
            }
            ExprKind::New(n) => {
                self.open("new");
                self.type_spec(&n.ty);
                for a in &n.args {
                    self.expr(a);
                }
                self.close();
            }
            ExprKind::Object(o) => {
                self.open("object");
                self.object_decl(&o.decl);
                self.block("body", &o.body);
                self.close();
            }
            ExprKind::MapCall(m) => {
                self.open(if m.xmap { "xmap" } else { "map" });
                self.source(&m.source);
                self.names("name", &m.name.segments);
                for a in &m.args {
                    self.expr(a);
                }
                self.close();
            }
            ExprKind::Resolve(r) => {
                self.open(kw(r.op));
                if r.late {
                    self.atom("late");
                }
                self.source(&r.source);
                if let Some(m) = &r.mapping {
                    self.scoped(m);
                }
                if let Some(t) = &r.target {
                    self.open("target");
                    self.opt_name(&t.var);
                    self.ty(&t.ty);
                    if let Some(c) = &t.cond {
                        self.expr(c);
                    }
                    self.close();
                }
                self.close();
            }
            ExprKind::Raise(r) => {
                self.open("raise");
                self.expr(&r.exception);
                match &r.arg {
                    None => {}
                    Some(None) => self.atom("()"),
                    Some(Some(a)) => self.expr(a),
                }
                self.close();
            }
            ExprKind::Try(t) => {
                self.open("try");
                self.block("body", &t.body);
                for c in &t.catches {
                    self.open("except");
                    self.opt_name(&c.var);
                    for ty in &c.types {
                        self.ty(ty);
                    }
                    self.block("body", &c.body);
                    self.close();
                }
                self.close();
            }
            ExprKind::Error => self.leaf("error", "expr"),
        }
    }
}
