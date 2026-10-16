//! End-to-end checks of small transformations (parse, analyse, compile, run) against Ecore:
//! the log and the outcome, with the values Eclipse QVTo gives for the same programs (checked
//! with the reference runner, `reference/runner/qvto-run --qvto t.qvto`).

use qvto_emf::resource_set::ResourceSet;
use qvto_sema::{UnitResolver, UnitSource};
use std::rc::Rc;

struct NoImports;

impl UnitResolver for NoImports {
    fn resolve(&self, _importer: &UnitSource, _q: &str) -> Option<UnitSource> {
        None
    }
}

/// Runs a transformation without model parameters (or with empty ones) in compat mode;
/// returns the log and the outcome's severity/code/message.
fn run(text: &str) -> (String, i32, i32, String) {
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
    let problems: Vec<String> = c
        .units
        .iter()
        .flat_map(|u| u.problems.iter().map(|p| p.message.clone()))
        .collect();
    let program = c
        .program
        .unwrap_or_else(|| panic!("no program: {problems:#?}"));
    let n = program.ir.modules[program.ir.main_module.index()]
        .model_params
        .len();
    let mut vm = qvto::Vm::new(&program, &mut rs.graph, qvto::RunOptions::default());
    let out = vm.run_transformation(vec![Vec::new(); n]);
    (vm.log.clone(), out.severity, out.code, out.message)
}

fn log_of(text: &str) -> String {
    let (log, sev, code, msg) = run(text);
    assert_eq!((sev, code), (0, 0), "{msg}\n{log}");
    log
}

#[test]
fn list_mutators_return_the_list() {
    // ListOperations' ADD/INSERT_AT_VOID/REMOVE/REMOVE_ALL are typed OclVoid but return the list
    let log = log_of(
        "transformation t();
         main() {
           var a : List(Integer) := List{1}->insertAt(2, 2);
           log('insertAt', a);
           var b : List(Integer) := List{1}->add(3);
           log('add', b);
           var c : List(Integer) := List{1, 2, 1}->remove(1);
           log('remove', c);
           var d : List(Integer) := List{1, 2, 3}->removeAll(Set{1, 3});
           log('removeAll', d);
         }",
    );
    assert_eq!(
        log,
        "insertAt, data: [1, 2]\nadd, data: [1, 3]\nremove, data: [2]\nremoveAll, data: [2]\n"
    );
}

#[test]
fn ocl_two_argument_operations_with_invalid_arguments() {
    // classic OCL's ternary operations: an invalid argument makes the call invalid
    let log = log_of(
        "transformation t();
         main() {
           log('seq', Sequence{1}->insertAt(1, invalid)->size());
           log('oset', OrderedSet{1}->insertAt(1, invalid)->size());
           log('sub', 'abc'.substring(1, invalid));
           log('seq2', Sequence{1, 2}->subSequence(invalid, 1));
         }",
    );
    assert_eq!(
        log,
        "seq, data: <Invalid>\noset, data: <Invalid>\nsub, data: <Invalid>\nseq2, data: <Invalid>\n"
    );
}
