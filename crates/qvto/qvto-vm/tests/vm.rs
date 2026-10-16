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
                    target_span: Default::default(),
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
                    target_span: Default::default(),
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
                        target_span: Default::default(),
                        target: LValue::Prop(local(a, ta), fa),
                        target_ty: S,
                        value: s("n"),
                        add: false,
                    })),
                    S,
                ),
                e(
                    ExprKind::Assign(Box::new(Assign {
                        target_span: Default::default(),
                        target: LValue::Prop(local(a, ta), fl),
                        target_ty: seq,
                        value: s("t1"),
                        add: true,
                    })),
                    S,
                ),
                e(
                    ExprKind::Assign(Box::new(Assign {
                        target_span: Default::default(),
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

#[test]
fn closure_iterate_xselect_xcollect() {
    // Sequence{1,2,3}->closure(x | if x < 5 then Sequence{x + 3} else Sequence{} endif)
    let mut t = Irb::new("");
    let m = t.entry;
    let seq = t.coll(CollKind::Sequence, I);
    let oset = t.coll(CollKind::OrderedSet, I);
    let x = t.local(m, "x", I, LocalKind::Iterator);
    let body = e(
        ExprKind::If(
            Box::new(bin(StdOwner::Integer, "<", local(x, I), int(5), B)),
            Box::new(coll_lit(
                CollKind::Sequence,
                vec![bin(StdOwner::Integer, "+", local(x, I), int(3), I)],
                seq,
            )),
            Some(Box::new(coll_lit(CollKind::Sequence, vec![], seq))),
        ),
        seq,
    );
    let clo = lp(
        LoopKind::Closure,
        coll_lit(CollKind::Sequence, vec![int(1), int(2), int(3)], seq),
        x,
        Some(body),
        None,
        oset,
    );
    // iterate(y; acc : Integer = 0 | acc + y)
    let y = t.local(m, "y", I, LocalKind::Iterator);
    let acc = t.local(m, "acc", I, LocalKind::Accumulator);
    let it = e(
        ExprKind::Loop(Box::new(Loop {
            kind: LoopKind::Iterate,
            source: coll_lit(CollKind::Sequence, vec![int(1), int(2), int(3)], seq),
            iterators: vec![y],
            acc: Some((acc, int(0))),
            target: None,
            body: Some(bin(StdOwner::Integer, "+", local(acc, I), local(y, I), I)),
            cond: None,
            filter_type: None,
        })),
        I,
    );
    // Sequence{1, null, 2}->xselect(z | z > 1) ; ->xcollect(w | if w = 1 then null else w endif)
    let z = t.local(m, "z", I, LocalKind::Iterator);
    let w = t.local(m, "w", I, LocalKind::Iterator);
    let src3 = || {
        coll_lit(
            CollKind::Sequence,
            vec![int(1), e(ExprKind::Null, I), int(2)],
            TypeId(0),
        )
    };
    let mut s3 = src3();
    s3.ty = seq;
    let xs = lp(
        LoopKind::XSelect,
        s3,
        z,
        None,
        Some(bin(StdOwner::Integer, ">", local(z, I), int(1), B)),
        seq,
    );
    let mut s4 = src3();
    s4.ty = seq;
    let xc = lp(
        LoopKind::XCollect,
        s4,
        w,
        Some(e(
            ExprKind::If(
                Box::new(bin(StdOwner::Integer, "=", local(w, I), int(1), B)),
                Box::new(e(ExprKind::Null, I)),
                Some(Box::new(local(w, I))),
            ),
            I,
        )),
        None,
        seq,
    );
    t.main(vec![
        log(repr(clo)),
        log(to_string(it)),
        log(repr(xs)),
        log(repr(xc)),
    ]);
    let (out, log) = t.run();
    assert!(out.error.is_none(), "{log}");
    // closure: depth-first: 1 -> 4 -> 7 (sources are not included), 2 -> 5, 3 -> 6
    assert_eq!(log, "[4, 7, 5, 6]\n6\n[2]\n[2]\n");
}

#[test]
fn mapping_out_params_inherits_disjuncts() {
    // mapping a(in i : Integer, out s : String) : String { init { s := 'out' + i.toString(); result := 'r' } }
    // mapping b(in i : Integer) : String when { i = 1 } { init { result := 'b' } }
    // mapping c(in i : Integer) : String when { i = 2 } { init { result := 'c' } }
    // mapping d(in i : Integer) : String disjuncts b, c;
    let mut t = Irb::new("");
    let ma = t.op("a", OpKind::Mapping, None, S);
    let ai = t.param(ma, "i", I, Direction::In);
    let as_ = t.param(ma, "s", S, Direction::Out);
    let ar = t.result(ma, S);
    t.mapping(ma).init = vec![
        assign(
            as_,
            bin(StdOwner::String, "+", s("out"), to_string(local(ai, I)), S),
            S,
        ),
        assign(ar, s("r"), S),
    ];
    let mk = |t: &mut Irb, name: &str, k: i64| {
        let mb = t.op(name, OpKind::Mapping, None, S);
        let bi = t.param(mb, "i", I, Direction::In);
        let br = t.result(mb, S);
        t.mapping(mb).when = vec![bin(StdOwner::Integer, "=", local(bi, I), int(k), B)];
        t.mapping(mb).init = vec![assign(br, s(name), S)];
        mb
    };
    let mb = mk(&mut t, "b", 1);
    let mc = mk(&mut t, "c", 2);
    let md = t.op("d", OpKind::Mapping, None, S);
    t.param(md, "i", I, Direction::In);
    t.result(md, S);
    t.mapping(md).disjuncts = vec![mb, mc];
    let m = t.entry;
    let v = t.var(m, "v", S);
    t.main(vec![
        var_init(v, None),
        log(map(ma, None, vec![int(7), local(v, S)], S, false)),
        log(local(v, S)),
        log(map(md, None, vec![int(2)], S, false)),
        log(map(md, None, vec![int(3)], S, false)),
        log(map(md, None, vec![int(2)], S, false)),
    ]);
    let (out, log) = t.run();
    assert!(out.error.is_none(), "{log}");
    assert_eq!(log, "r\nout7\nc\n<null>\nc\n");
    // a, c, d (for 2) ; the second call d(2) is fetched from the trace
    assert_eq!(out.trace_records, 3);
}

#[test]
fn virtual_dispatch_on_context() {
    // query A::name() : String { 'A' }  query B::name() : String { 'B' } (B extends A)
    let mut t = Irb::new("");
    let pkg = t.package("vd");
    let ca = t.class(pkg, "A", &[]);
    let cb = t.class(pkg, "B", &[ca]);
    t.finish_meta();
    let _ = t.model_param("out", pkg, Direction::Out);
    let ta = t.class_ty(ca);
    let tb = t.class_ty(cb);
    let qa = t.op("name", OpKind::Query, Some(ta), S);
    t.body(qa, vec![s("A")]);
    let qb = t.op("name", OpKind::Query, Some(tb), S);
    t.body(qb, vec![s("B")]);
    t.p.operations[qa.index()].vtable = vec![qb];
    t.p.operations[qb.index()].vtable = vec![qa];
    let m = t.entry;
    let o = t.var(m, "o", ta);
    let obj = e(
        ExprKind::Object(Box::new(ObjectExp {
            var: o,
            class: cb,
            extent: None,
            body: vec![],
        })),
        ta,
    );
    let vcall = e(
        ExprKind::Call(Box::new(Call {
            callee: Callee::Op {
                op: qa,
                virtual_: true,
            },
            source: Some(local(o, ta)),
            args: vec![],
        })),
        S,
    );
    t.main(vec![var_init(o, Some(obj)), log(vcall)]);
    let (out, log) = t.run();
    assert!(out.error.is_none(), "{log}");
    assert_eq!(log, "B\n");
}

#[test]
fn strings_and_regex() {
    let mut t = Irb::new("");
    t.main(vec![
        log(std(
            sop(StdOwner::String, "substring", 2),
            Some(s("hello")),
            vec![int(2), int(4)],
            S,
        )),
        log(to_string(std(
            sop(StdOwner::String, "indexOf", 1),
            Some(s("hello")),
            vec![s("l")],
            I,
        ))),
        log(std(
            sop(StdOwner::String, "toUpper", 0),
            Some(s("abc")),
            vec![],
            S,
        )),
        log(to_string(std(
            sop(StdOwner::String, "matches", 1),
            Some(s("abc123")),
            vec![s("[a-z]+[0-9]+")],
            B,
        ))),
        log(std(
            sop(StdOwner::String, "replaceAll", 2),
            Some(s("a-b-c")),
            vec![s("-"), s("+")],
            S,
        )),
        log(std(
            sop(StdOwner::String, "firstToUpper", 0),
            Some(s("name")),
            vec![],
            S,
        )),
    ]);
    let (_, log) = t.run();
    assert_eq!(log, "ell\n3\nABC\ntrue\na+b+c\nName\n");
}
