//! VM tests on hand-written typed IR (what `qvto-sema` produces for the quoted QVTo).

mod common;

use common::*;
use qvto_model::ModelStore;
use qvto_sema::ir::*;
use qvto_types::{CollKind, StdOwner, Type, TypeId};
use qvto_vm::RunOptions;

const I: TypeId = TypeId::INTEGER;
const S: TypeId = TypeId::STRING;
const B: TypeId = TypeId::BOOLEAN;
const V: TypeId = TypeId::OCL_VOID;

#[test]
fn arithmetic_and_strings() {
    // log((1 + 2).toString()); log((7 / 2).toString()); log('a' + 'b'); log((10 div 3).repr())
    let mut t = Irb::new("");
    t.main(vec![
        log(to_string(bin(StdOwner::Integer, "+", int(1), int(2), I))),
        log(to_string(bin(
            StdOwner::Integer,
            "/",
            int(7),
            int(2),
            TypeId::REAL,
        ))),
        log(bin(StdOwner::String, "+", s("a"), s("b"), S)),
        log(repr(bin(StdOwner::Integer, "div", int(10), int(3), I))),
        log(to_string(bin(
            StdOwner::Integer,
            "*",
            int(1 << 40),
            int(1 << 30),
            I,
        ))),
        log(to_string(bin(
            StdOwner::Integer,
            "/",
            int(1),
            int(0),
            TypeId::REAL,
        ))),
    ]);
    let (out, log) = t.run();
    assert_eq!(out.severity, 0, "{out:?}");
    assert_eq!(log, "3\n3.5\nab\n3\n0\ninvalid\n");
}

#[test]
fn variables_if_while_switch() {
    // var i := 0; var acc := ''; while (i < 5) { i := i + 1; if (i = 3) then continue endif; acc := acc + i.toString() }
    let mut t = Irb::new("");
    let m = t.entry;
    let i = t.var(m, "i", I);
    let acc = t.var(m, "acc", S);
    let cond = bin(StdOwner::Integer, "<", local(i, I), int(5), B);
    let body = block(vec![
        assign(i, bin(StdOwner::Integer, "+", local(i, I), int(1), I), I),
        e(
            ExprKind::If(
                Box::new(bin(StdOwner::Integer, "=", local(i, I), int(3), B)),
                Box::new(e(ExprKind::Continue, V)),
                None,
            ),
            V,
        ),
        assign(
            acc,
            bin(
                StdOwner::String,
                "+",
                local(acc, S),
                to_string(local(i, I)),
                S,
            ),
            S,
        ),
    ]);
    let sw = e(
        ExprKind::Switch(
            vec![
                (
                    bin(StdOwner::Integer, "<", local(i, I), int(3), B),
                    s("small"),
                ),
                (
                    bin(StdOwner::Integer, "=", local(i, I), int(5), B),
                    s("five"),
                ),
            ],
            Some(Box::new(s("other"))),
        ),
        S,
    );
    t.main(vec![
        var_init(i, Some(int(0))),
        var_init(acc, None),
        e(ExprKind::While(Box::new(cond), Box::new(body)), V),
        log(local(acc, S)),
        log(sw),
        // if with an undefined condition is invalid
        log(e(
            ExprKind::If(
                Box::new(e(ExprKind::Null, B)),
                Box::new(s("x")),
                Some(Box::new(s("y"))),
            ),
            S,
        )),
    ]);
    let (_, log) = t.run();
    assert_eq!(log, "1245\nfive\n<Invalid>\n");
}

#[test]
fn boolean_lax_null() {
    // true and null -> invalid; false and null -> false; null or true -> true
    let mut t = Irb::new("");
    let nb = || e(ExprKind::Null, B);
    t.main(vec![
        log(to_string(bin(StdOwner::Boolean, "and", b(true), nb(), B))),
        log(to_string(bin(StdOwner::Boolean, "and", b(false), nb(), B))),
        log(to_string(bin(StdOwner::Boolean, "or", nb(), b(true), B))),
        log(to_string(bin(
            StdOwner::Boolean,
            "implies",
            b(true),
            b(false),
            B,
        ))),
    ]);
    let (_, log) = t.run();
    assert_eq!(log, "invalid\nfalse\ntrue\nfalse\n");
}

#[test]
fn iterators() {
    // Sequence{3,1,2}->select(x | x > 1)->collect(x | x * 10)->sortedBy(x | x)
    let mut t = Irb::new("");
    let m = t.entry;
    let seq = t.coll(CollKind::Sequence, I);
    let x = t.local(m, "x", I, LocalKind::Iterator);
    let y = t.local(m, "y", I, LocalKind::Iterator);
    let z = t.local(m, "z", I, LocalKind::Iterator);
    let w = t.local(m, "w", I, LocalKind::Iterator);
    let src = coll_lit(CollKind::Sequence, vec![int(3), int(1), int(2)], seq);
    let sel = lp(
        LoopKind::Select,
        src,
        x,
        Some(bin(StdOwner::Integer, ">", local(x, I), int(1), B)),
        None,
        seq,
    );
    let col = lp(
        LoopKind::Collect,
        sel,
        y,
        Some(bin(StdOwner::Integer, "*", local(y, I), int(10), I)),
        None,
        seq,
    );
    let sorted = lp(LoopKind::SortedBy, col, z, Some(local(z, I)), None, seq);
    let ex = lp(
        LoopKind::Exists,
        coll_lit(CollKind::Sequence, vec![int(1), int(2)], seq),
        w,
        Some(bin(StdOwner::Integer, "=", local(w, I), int(2), B)),
        None,
        B,
    );
    t.main(vec![log(repr(sorted)), log(to_string(ex))]);
    let (_, log) = t.run();
    assert_eq!(log, "[20, 30]\ntrue\n");
}

#[test]
fn helper_calls_and_stack_overflow() {
    // helper fact(n : Integer) : Integer { if n <= 1 then 1 else n * fact(n - 1) endif }
    let mut t = Irb::new("main\nfact\n");
    let f = t.op("fact", OpKind::Helper, None, I);
    let n = t.param(f, "n", I, Direction::In);
    set_pos(5);
    let rec = call(
        f,
        None,
        vec![bin(StdOwner::Integer, "-", local(n, I), int(1), I)],
        I,
    );
    t.body(
        f,
        vec![at(
            e(
                ExprKind::If(
                    Box::new(bin(StdOwner::Integer, "<=", local(n, I), int(1), B)),
                    Box::new(int(1)),
                    Some(Box::new(bin(StdOwner::Integer, "*", local(n, I), rec, I))),
                ),
                I,
            ),
            5,
        )],
    );
    set_pos(0);
    t.main(vec![
        log(to_string(call(f, None, vec![int(5)], I))),
        log(to_string(call(f, None, vec![int(100)], I))),
    ]);
    let opts = RunOptions {
        max_stack_depth: 10,
        ..RunOptions::default()
    };
    let (out, log) = t.run_with(opts, vec![]);
    let err = out.error.expect("stack overflow");
    assert_eq!(
        err.java_class(),
        "org.eclipse.m2m.internal.qvt.oml.evaluator.QvtStackOverFlowError"
    );
    assert_eq!(out.message, "QVT Stack overflow occured during execution");
    assert_eq!(out.code, 120);
    assert!(log.starts_with("120\nTerminating execution...\norg.eclipse.m2m.internal.qvt.oml.evaluator.QvtStackOverFlowError: Stack depth: 10\n\tat test::fact(test.qvto:2)\n"), "{log}");
    assert_eq!(err.stack.len(), 9);
    assert_eq!(
        err.stack.last().unwrap().to_string(),
        "test::main(test.qvto:1)"
    );
}

#[test]
fn raise_and_try() {
    // try { log('a'); raise Exception('boom'); log('b') } except (Exception e) { log(e.getArgument()) }; log('c')
    let mut t = Irb::new("");
    let m = t.entry;
    let exc_class = t.p.std_classes.exception;
    let ety = t.class_ty(exc_class);
    let ev = t.local(m, "e", ety, LocalKind::Catch);
    let get_arg = std(
        sop(StdOwner::Exception, "getArgument", 0),
        Some(local(ev, ety)),
        vec![],
        S,
    );
    let tr = e(
        ExprKind::Try(Box::new(Try {
            body: block(vec![
                log(s("a")),
                e(
                    ExprKind::Raise(Some(exc_class), Some(Box::new(s("boom")))),
                    V,
                ),
                log(s("b")),
            ]),
            catches: vec![Catch {
                types: vec![exc_class],
                var: Some(ev),
                body: block(vec![log(get_arg)]),
                span: sp(0),
            }],
        })),
        V,
    );
    t.main(vec![tr, log(s("c"))]);
    let (out, log) = t.run();
    assert!(out.error.is_none(), "{out:?}");
    assert_eq!(log, "a\nboom\nc\n");
}

#[test]
fn fatal_assert() {
    let mut t = Irb::new("main\n  assert fatal (false) with log('no');\n");
    set_pos(7);
    let a = e(
        ExprKind::Assert(Box::new(Assert {
            severity: AssertSeverity::Fatal,
            cond: b(false),
            log: Some(Log {
                message: Some(s("no")),
                element: None,
                level: None,
                when: None,
            }),
        })),
        V,
    );
    let mut a = a;
    a.span = sp(7);
    set_pos(0);
    t.main(vec![a, log(s("not reached"))]);
    let (out, log) = t.run();
    assert_eq!(out.code, 100);
    assert_eq!(out.message, "no");
    assert_eq!(
        log,
        "ASSERT [fatal] failed at (test.qvto:2) : no\nTerminating execution...\norg.eclipse.m2m.internal.qvt.oml.evaluator.QvtAssertionFailed: no\n\tat test::main(test.qvto:2)\n"
    );
}

#[test]
fn mappings_objects_traces_resolve() {
    // metamodel: A { name : EString }, B { name : EString }
    // mapping A::toB() : B { result.name := self.name + '!' }
    // main: var a := object A { name := 'x' }; var b1 := a.map toB(); var b2 := a.map toB();
    //       log((b1 = b2).toString()); log(b1.name); log(a.resolveone(B).name)
    let mut t = Irb::new("");
    let pkg = t.package("mm");
    let ca = t.class(pkg, "A", &[]);
    let cb = t.class(pkg, "B", &[]);
    let fa = t.attr(ca, "name", "EString", false);
    let fbn = t.attr(cb, "name", "EString", false);
    t.finish_meta();
    let out_param = t.model_param("out", pkg, Direction::Out);
    let ta = t.class_ty(ca);
    let tb = t.class_ty(cb);
    let mp = t.op("toB", OpKind::Mapping, Some(ta), tb);
    let r = t.result(mp, tb);
    let selfl = t.p.operations[mp.index()].self_local.unwrap();
    let pop = e(
        ExprKind::Object(Box::new(ObjectExp {
            var: r,
            class: cb,
            extent: None,
            body: vec![e(
                ExprKind::Assign(Box::new(Assign {
                    target: LValue::Prop(local(r, tb), fbn),
                    target_ty: S,
                    value: bin(
                        StdOwner::String,
                        "+",
                        e(ExprKind::Prop(Box::new(local(selfl, ta)), fa), S),
                        s("!"),
                        S,
                    ),
                    add: false,
                })),
                S,
            )],
        })),
        tb,
    );
    t.mapping(mp).population = vec![pop];
    let m = t.entry;
    let a = t.var(m, "a", ta);
    let b1 = t.var(m, "b1", tb);
    let b2 = t.var(m, "b2", tb);
    let obj_a = e(
        ExprKind::Object(Box::new(ObjectExp {
            var: a,
            class: ca,
            extent: Some(out_param),
            body: vec![e(
                ExprKind::Assign(Box::new(Assign {
                    target: LValue::Prop(local(a, ta), fa),
                    target_ty: S,
                    value: s("x"),
                    add: false,
                })),
                S,
            )],
        })),
        ta,
    );
    let resolved = e(
        ExprKind::Resolve(Box::new(Resolve {
            kind: ResolveKind::ResolveOne,
            late: false,
            source: Some(local(a, ta)),
            mapping: None,
            target_ty: Some(tb),
            cond_var: None,
            cond: None,
        })),
        tb,
    );
    t.main(vec![
        var_init(a, Some(obj_a)),
        var_init(b1, Some(map(mp, Some(local(a, ta)), vec![], tb, false))),
        var_init(b2, Some(map(mp, Some(local(a, ta)), vec![], tb, false))),
        log(to_string(bin(
            StdOwner::OclAny,
            "=",
            local(b1, tb),
            local(b2, tb),
            B,
        ))),
        log(e(ExprKind::Prop(Box::new(local(b1, tb)), fbn), S)),
        log(e(ExprKind::Prop(Box::new(resolved), fbn), S)),
    ]);
    let (out, log) = t.run();
    assert!(out.error.is_none(), "{out:?} {log}");
    assert_eq!(log, "true\nx!\nx!\n");
    assert_eq!(out.trace_records, 1);
    // the out extent: A and B as roots, in creation order
    assert_eq!(out.extents[0].as_ref().unwrap().len(), 2);
}

#[test]
fn when_guard_and_xmap() {
    // mapping m(i : Integer) : String when { i > 0 } { init { result := 'ok' } }
    let mut t = Irb::new("");
    let mp = t.op("m", OpKind::Mapping, None, S);
    let i = t.param(mp, "i", I, Direction::In);
    let r = t.result(mp, S);
    t.mapping(mp).when = vec![bin(StdOwner::Integer, ">", local(i, I), int(0), B)];
    t.mapping(mp).init = vec![assign(r, s("ok"), S)];
    t.main(vec![
        log(map(mp, None, vec![int(1)], S, false)),
        log(map(mp, None, vec![int(-1)], S, false)),
        log(map(mp, None, vec![int(-1)], S, true)),
    ]);
    let (out, log) = t.run();
    assert_eq!(out.code, 100, "{log}");
    assert_eq!(out.message, "Precondition failed in mapping 'm'");
    assert!(log.starts_with("ok\n<null>\nTerminating"), "{log}");
}

#[test]
fn collections_kinds_and_dict_list() {
    let mut t = Irb::new("");
    let m = t.entry;
    let set_s = t.coll(CollKind::Set, S);
    let list_i = t.ty(Type::List(I));
    let dict = t.ty(Type::Dict(S, I));
    let l = t.var(m, "l", list_i);
    let d = t.var(m, "d", dict);
    let add = std(
        sop(StdOwner::List, "add", 1),
        Some(local(l, list_i)),
        vec![int(5)],
        V,
    );
    let put = std(
        sop(StdOwner::Dict, "put", 2),
        Some(local(d, dict)),
        vec![s("k"), int(7)],
        V,
    );
    let get = std(
        sop(StdOwner::Dict, "get", 1),
        Some(local(d, dict)),
        vec![s("k")],
        I,
    );
    t.main(vec![
        log(repr(coll_lit(
            CollKind::Set,
            vec![s("b"), s("a"), s("b")],
            set_s,
        ))),
        var_init(l, None),
        add,
        log(repr(local(l, list_i))),
        var_init(d, None),
        put,
        log(to_string(get)),
    ]);
    let (_, log) = t.run();
    assert_eq!(log, "[a, b]\n[5]\n7\n");
}

#[test]
fn objects_in_out_extent_and_store() {
    // object A { name := 'n' } into the out extent; the store holds it as a root of extent:/1
    let mut t = Irb::new("");
    let pkg = t.package("mm2");
    let ca = t.class(pkg, "A", &[]);
    let fa = t.attr(ca, "name", "EString", false);
    let fl = t.attr(ca, "tags", "EString", true);
    t.finish_meta();
    let _ = t.model_param("out", pkg, Direction::Out);
    let ta = t.class_ty(ca);
    let m = t.entry;
    let a = t.var(m, "a", ta);
    let seq = t.coll(CollKind::OrderedSet, S);
    let obj = e(
        ExprKind::Object(Box::new(ObjectExp {
            var: a,
            class: ca,
            extent: None,
            body: vec![
                e(
                    ExprKind::Assign(Box::new(Assign {
                        target: LValue::Prop(local(a, ta), fa),
                        target_ty: S,
                        value: s("n"),
                        add: false,
                    })),
                    S,
                ),
                e(
                    ExprKind::Assign(Box::new(Assign {
                        target: LValue::Prop(local(a, ta), fl),
                        target_ty: seq,
                        value: s("t1"),
                        add: true,
                    })),
                    S,
                ),
                e(
                    ExprKind::Assign(Box::new(Assign {
                        target: LValue::Prop(local(a, ta), fl),
                        target_ty: seq,
                        value: s("t2"),
                        add: true,
                    })),
                    S,
                ),
            ],
        })),
        ta,
    );
    t.main(vec![
        var_init(a, Some(obj)),
        log(repr(e(ExprKind::Prop(Box::new(local(a, ta)), fl), seq))),
    ]);
    let graph_check;
    let (out, log) = {
        let prog = qvto_bytecode::compile(t.p).unwrap();
        let mut vm = qvto_vm::Vm::new(&prog, &mut t.graph, RunOptions::default());
        let o = vm.run_transformation(vec![vec![]]);
        let l = vm.log.clone();
        graph_check = Some(t.graph.resource_count());
        (o, l)
    };
    assert_eq!(log, "[t1, t2]\n");
    let roots = out.extents[0].as_ref().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(graph_check, Some(1));
}
