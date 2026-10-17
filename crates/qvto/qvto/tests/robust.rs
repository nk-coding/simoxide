//! Inputs found by the robustness fuzzer (`qvto fuzz robust`, PLAN Q7), minimised: the whole
//! pipeline (parse, analyse, compile, run with a step budget) must report problems or results,
//! never panic, hang or exhaust memory.

use qvto_emf::resource_set::ResourceSet;
use qvto_sema::{UnitResolver, UnitSource};
use std::rc::Rc;

struct NoImports;

impl UnitResolver for NoImports {
    fn resolve(&self, _importer: &UnitSource, _q: &str) -> Option<UnitSource> {
        None
    }
}

/// Compiles (and runs, if it compiles) `text`; returns whether it compiled without errors.
fn pipeline(text: &str) -> bool {
    let mut rs = ResourceSet::new();
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let main = UnitSource {
        key: "mem:t".into(),
        qualified_name: "t".into(),
        file_name: "t.qvto".into(),
        path: None,
        text: Rc::from(text),
        blackbox: None,
    };
    let c = qvto::compile(main, &NoImports, meta);
    let Some(program) = c.program else {
        return false;
    };
    let n = program.ir.modules[program.ir.main_module.index()]
        .model_params
        .len();
    let opts = qvto::RunOptions {
        step_limit: Some(1_000_000),
        ..qvto::RunOptions::default()
    };
    let mut vm = qvto::Vm::new(&program, &mut rs.graph, opts);
    let _ = vm.run_transformation(vec![Vec::new(); n]);
    true
}

/// Syntax errors that leave nodes without names (mapping calls, module references).
#[test]
fn nameless_nodes_after_syntax_errors() {
    for src in [
        "transformation\n\tmain{map",
        "transformation main{xmap",
        "transformation\nmapping{self>map",
        "transformation a() extends ;",
        "transformation a() access ;",
        "transformation a() extends transformation ;",
        "library l; transformation a() extends l, ;",
        "transformation a() access library;",
    ] {
        assert!(!pipeline(src), "{src}");
    }
}

/// Operations that would build huge values stop at the run's limits (default limits here).
#[test]
fn resource_limits() {
    for src in [
        "transformation t(); main() { var c : List(Integer) = List{2147483648 .. 3}; log('x', c->size()); }",
        "transformation t(); main() { log('x', Sequence{1..2000000000}->size()); }",
        "transformation t(); main() { var c := Sequence{1..20000}; log('x', c->product(c)->size()); }",
        "transformation t(); main() { var s := 'ab'; while (true) { s := s + s; }; }",
        "transformation t(); main() { while (true) { }; }",
        "transformation t(); helper f(i : Integer) : Integer { return f(i + 1); } main() { log('x', f(1)); }",
    ] {
        assert!(pipeline(src), "{src}");
    }
}
