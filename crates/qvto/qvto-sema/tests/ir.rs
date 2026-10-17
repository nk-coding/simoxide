//! Checks of the typed IR the analyser produces for small programs (Q4b): the facts the VM
//! relies on, beyond the diagnostics that `tests/oracle.rs` compares with Eclipse.

use qvto_sema::ir::{self, Direction, ExprKind, OpId, Program};
use qvto_sema::{Compilation, UnitResolver, UnitSource, compile};
use std::rc::Rc;

/// Resolves imports from a list of `(qualified name, text)` pairs.
struct Mem(Vec<(&'static str, &'static str)>);

fn unit(qname: &str, text: &str) -> UnitSource {
    UnitSource {
        key: format!("mem:{qname}"),
        qualified_name: qname.to_string(),
        file_name: format!("{}.qvto", qname.rsplit('.').next().unwrap()),
        path: None,
        text: Rc::from(text),
        blackbox: None,
    }
}

impl UnitResolver for Mem {
    fn resolve(&self, _importer: &UnitSource, q: &str) -> Option<UnitSource> {
        self.0.iter().find(|(n, _)| *n == q).map(|(n, t)| unit(n, t))
    }
}

fn compile_units(main: &str, others: Vec<(&'static str, &'static str)>) -> Compilation {
    let mut rs = qvto_emf::resource_set::ResourceSet::new();
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let c = compile(unit("main", main), &Mem(others), meta);
    let problems: Vec<String> = c
        .units
        .iter()
        .flat_map(|u| u.problems.iter().map(|p| format!("{}: {}", u.source.file_name, p.message)))
        .collect();
    assert!(!c.has_errors(), "errors: {problems:#?}");
    c
}

fn compile_ok(main: &str) -> Program {
    compile_units(main, Vec::new()).program
}

fn op_named(p: &Program, module: &str, name: &str) -> OpId {
    let i = p
        .operations
        .iter()
        .position(|o| o.name == name && p.modules[o.module.index()].name == module)
        .unwrap_or_else(|| panic!("no operation {module}::{name}"));
    OpId(i as u32)
}

fn walk<'a>(e: &'a ir::Expr, f: &mut dyn FnMut(&'a ir::Expr)) {
    f(e);
    use ExprKind as K;
    match &e.kind {
        K::Coll(_, parts) | K::List(parts) => {
            for p in parts {
                match p {
                    ir::CollPart::Item(x) => walk(x, f),
                    ir::CollPart::Range(a, b) => {
                        walk(a, f);
                        walk(b, f);
                    }
                }
            }
        }
        K::Dict(kv) => {
            for (k, v) in kv {
                walk(k, f);
                walk(v, f);
            }
        }
        K::Tuple(ps) => ps.iter().for_each(|(_, x)| walk(x, f)),
        K::Prop(x, _) | K::TupleProp(x, _) | K::IntermProp(x, _) => walk(x, f),
        K::Call(c) => {
            if let Some(s) = &c.source {
                walk(s, f);
            }
            c.args.iter().for_each(|a| walk(a, f));
        }
        K::Loop(l) => {
            walk(&l.source, f);
            if let Some((_, a)) = &l.acc {
                walk(a, f);
            }
            if let Some(b) = &l.body {
                walk(b, f);
            }
            if let Some(c) = &l.cond {
                walk(c, f);
            }
        }
        K::Let(_, a, b) => {
            walk(a, f);
            walk(b, f);
        }
        K::If(c, a, b) => {
            walk(c, f);
            walk(a, f);
            if let Some(b) = b {
                walk(b, f);
            }
        }
        K::Block(xs) => xs.iter().for_each(|x| walk(x, f)),
        K::VarInit { init: Some(x), .. } => walk(x, f),
        K::Assign(a) => walk(&a.value, f),
        K::While(a, b) => {
            walk(a, f);
            walk(b, f);
        }
        K::Compute(_, i, b) => {
            if let Some(i) = i {
                walk(i, f);
            }
            walk(b, f);
        }
        K::Return(Some(x)) => walk(x, f),
        K::Raise(_, Some(x)) => walk(x, f),
        K::Try(t) => {
            walk(&t.body, f);
            t.catches.iter().for_each(|c| walk(&c.body, f));
        }
        K::Object(o) => o.body.iter().for_each(|x| walk(x, f)),
        K::New(n) => n.args.iter().for_each(|x| walk(x, f)),
        K::NewTransformation(_, xs) => xs.iter().for_each(|x| walk(x, f)),
        _ => {}
    }
}

fn body_exprs(p: &Program, op: OpId) -> Vec<&ir::Expr> {
    let o = &p.operations[op.index()];
    let mut out = Vec::new();
    let mut roots: Vec<&ir::Expr> = Vec::new();
    if let Some(b) = &o.body {
        roots.extend(b.iter());
    }
    if let Some(m) = &o.mapping {
        for s in [&m.when, &m.where_, &m.init, &m.population, &m.end] {
            roots.extend(s.iter());
        }
    }
    for r in roots {
        walk(r, &mut |e| out.push(e));
    }
    out
}

#[test]
fn model_params_are_in_the_ir() {
    let p = compile_ok(
        "modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         transformation t(in a : ECORE, out b : ECORE, inout c : ECORE);
         main() {}",
    );
    let m = &p.modules[p.main_module.index()];
    let got: Vec<(&str, Direction, bool)> = m
        .model_params
        .iter()
        .map(|mp| (mp.name.as_str(), mp.direction, mp.generated))
        .collect();
    assert_eq!(
        got,
        vec![
            ("a", Direction::In, false),
            ("b", Direction::Out, false),
            ("c", Direction::InOut, false)
        ]
    );
    for mp in &m.model_params {
        assert_eq!(p.model_types[mp.model_type.index()].name, "ECORE");
    }
}

#[test]
fn legacy_main_parameters_are_generated() {
    let p = compile_ok(
        "modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         transformation t;
         mapping main(in x : ecore::EPackage) : ecore::EPackage {}",
    );
    let m = &p.modules[p.main_module.index()];
    let got: Vec<(&str, Direction, bool)> = m
        .model_params
        .iter()
        .map(|mp| (mp.name.as_str(), mp.direction, mp.generated))
        .collect();
    assert_eq!(
        got,
        vec![("$x", Direction::In, true), ("$result", Direction::Out, true)]
    );
}

#[test]
fn virtual_tables_are_in_the_ir() {
    let p = compile_ok(
        "modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         transformation t(in a : ECORE);
         main() {}
         helper ecore::EClassifier::f() : String { return 'classifier'; }
         helper ecore::EClass::f() : String { return 'class'; }
         helper ecore::EDataType::f() : String { return 'datatype'; }
         helper ecore::EPackage::f() : String { return 'package'; }",
    );
    let names = |op: OpId| -> Vec<String> {
        p.operations[op.index()]
            .vtable
            .iter()
            .map(|o| {
                let c = p.operations[o.index()].context.unwrap();
                format!("{:?}", p.types.get(c))
            })
            .collect()
    };
    let ops: Vec<OpId> = (0..p.operations.len() as u32)
        .map(OpId)
        .filter(|o| p.operations[o.index()].name == "f")
        .collect();
    assert_eq!(ops.len(), 4);
    // EClassifier's table has both subtypes' operations, each subtype's has EClassifier's;
    // EPackage is unrelated
    assert_eq!(p.operations[ops[0].index()].vtable, vec![ops[1], ops[2]], "{:?}", names(ops[0]));
    assert_eq!(p.operations[ops[1].index()].vtable, vec![ops[0]]);
    assert_eq!(p.operations[ops[2].index()].vtable, vec![ops[0]]);
    assert!(p.operations[ops[3].index()].vtable.is_empty());
}

#[test]
fn overriding_operations_of_extended_modules() {
    let c = compile_units(
        "import lib;
         modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         transformation t(in a : ECORE) extends lib;
         main() {}
         helper g() : String { return 't'; }",
        vec![(
            "lib",
            "modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
             transformation lib(in a : ECORE);
             helper g() : String { return 'lib'; }",
        )],
    );
    let p = &c.program;
    let g_t = op_named(p, "t", "g");
    let g_lib = op_named(p, "lib", "g");
    assert_eq!(p.operations[g_t.index()].overrides, Some(g_lib));
    assert_eq!(p.operations[g_lib.index()].overrides, None);
}

#[test]
fn raise_string_is_raise_exception() {
    // bug419299: `raise 'msg'` raises `Exception('msg')`, which `except (Exception)` catches
    let p = compile_ok(
        "transformation t();
         main() { try { raise 'boom'; } except (e : Exception) { log(e.getArgument()); }; }",
    );
    let main = op_named(&p, "t", "main");
    let raises: Vec<_> = body_exprs(&p, main)
        .into_iter()
        .filter_map(|e| match &e.kind {
            ExprKind::Raise(c, a) => Some((*c, a.as_deref().map(|a| a.kind.clone()))),
            _ => None,
        })
        .collect();
    assert_eq!(raises.len(), 1);
    let (class, arg) = &raises[0];
    let class = class.expect("raise without exception type");
    assert_eq!(&*p.meta.class(class).name, "Exception");
    assert!(matches!(arg, Some(ExprKind::Str(s)) if &**s == "boom"), "{arg:?}");
}

#[test]
fn iterate_with_an_implicit_iterator() {
    // iteratetest: `iterate(acc : T = init | body)`, the single declaration is the result and
    // the iterator is implicit (an implicit source of `name`)
    let p = compile_ok(
        "modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         transformation t(in a : ECORE);
         main() {
           var tmp := a.rootObjects()![ecore::EPackage].eClassifiers->iterate(s : OrderedSet(String) = OrderedSet {} |
             s->including(name)->asOrderedSet());
         }",
    );
    let main = op_named(&p, "t", "main");
    let o = &p.operations[main.index()];
    let loops: Vec<&ir::Loop> = body_exprs(&p, main)
        .into_iter()
        .filter_map(|e| match &e.kind {
            ExprKind::Loop(l) if l.kind == ir::LoopKind::Iterate => Some(&**l),
            _ => None,
        })
        .collect();
    assert_eq!(loops.len(), 1, "{:#?}", o.body);
    let l = loops[0];
    let (acc, _) = l.acc.as_ref().expect("accumulator");
    assert_eq!(o.locals[acc.index()].name, "s");
    assert_eq!(o.locals[acc.index()].kind, ir::LocalKind::Accumulator);
    assert_eq!(l.iterators.len(), 1);
    assert_eq!(o.locals[l.iterators[0].index()].kind, ir::LocalKind::Temp);
}

/// The problems of a compilation as `line: message`, in report order.
fn problems(main: &str) -> Vec<String> {
    let mut rs = qvto_emf::resource_set::ResourceSet::new();
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let c = compile(unit("main", main), &Mem(Vec::new()), meta);
    let u = &c.units[0];
    u.problems
        .iter()
        .map(|p| format!("{}: {}", p.line(&u.lines), p.message))
        .collect()
}

#[test]
fn validator_problems_inside_object_bodies_are_dropped() {
    // UMLFoo, Ecore2UML: OCL's validator reports at the CST node of the AST node, but object
    // bodies are analysed in an environment whose AST-to-CST map the validation does not see,
    // and QVTo drops validator problems without a position (checked with Eclipse)
    let p = problems(
        "modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         transformation w(inout m : ECORE);
         main() {
           var i := object Foo {};
           var k := i;
           object EClass { var q := i; };
           do { var q2 := i; };
         }",
    );
    assert_eq!(
        p,
        vec![
            "4: Unknown type (Foo)",
            "5: Incomplete reference to variable declaration in variable expression: (i)",
            "7: Incomplete reference to variable declaration in variable expression: (i)",
        ]
    );
}

#[test]
fn parser_recovery_shapes_as_eclipse() {
    // `a-b` is repaired to `a` (exec_erroneous): the name after the substituted `.` is dropped
    let p = problems("import foo-bar.baz;\ntransformation a-b();\nmain() {}");
    assert_eq!(
        p,
        vec![
            "1: \".\" expected instead of \"-\"",
            "2: \".\" expected instead of \"-\"",
            "1: Cannot find imported compilation unit 'foo.baz'",
        ]
    );
    // `object { ... }` without a type leaves nothing to analyse (missingObjectType)
    let p = problems(
        "transformation t;
         modeltype ECORE uses 'http://www.eclipse.org/emf/2002/Ecore';
         mapping m(): ECORE::EPackage { name := object { fff := 'a'; } }",
    );
    assert_eq!(p, vec!["3: \"objectDeclCS\" expected after \"object\""]);
}
