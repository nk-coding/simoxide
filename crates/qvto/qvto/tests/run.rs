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

const ANNOTATED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="A" nsURI="urn:annotated" nsPrefix="a">
  <eClassifiers xsi:type="ecore:EClass" name="Base">
    <eOperations name="op" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString">
      <eAnnotations source="http://www.eclipse.org/2007/OCL"><details key="body" value="'Base::op ' + p"/></eAnnotations>
      <eParameters name="p" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
    </eOperations>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="name" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
  </eClassifiers>
  <eClassifiers xsi:type="ecore:EClass" name="Sub" eSuperTypes="#//Base">
    <eOperations name="op" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString">
      <eAnnotations source="http://www.eclipse.org/2007/OCL"><details key="body" value="'Sub::op ' + p + ' ' + name"/></eAnnotations>
      <eParameters name="p" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString"/>
    </eOperations>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="upper" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString" derived="true">
      <eAnnotations source="http://www.eclipse.org/2007/OCL"><details key="derive" value="name.toUpper()"/></eAnnotations>
    </eStructuralFeatures>
    <eStructuralFeatures xsi:type="ecore:EAttribute" name="broken" eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString" derived="true">
      <eAnnotations source="http://www.eclipse.org/2007/OCL"><details key="derive" value="nothing.here"/></eAnnotations>
    </eStructuralFeatures>
  </eClassifiers>
</ecore:EPackage>"##;

#[test]
fn ocl_annotation_bodies_and_derivations() {
    // `OCLAnnotationSupport`: bodies of operations of dynamic classes, dispatched on the
    // instance's class; derived attributes evaluate their derivation; unparseable ones are
    // invalid (checked with Eclipse on the `oclannotation` case's metamodel)
    let mut rs = ResourceSet::new();
    let _ = rs.ecore_resource();
    let r = rs.load_text("file:/a.ecore", ANNOTATED).unwrap();
    rs.register(r).unwrap();
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let ders = qvto_emf::operations::meta_derivations(&rs.graph);
    let main = UnitSource {
        key: "mem:t".into(),
        qualified_name: "t".into(),
        file_name: "t.qvto".into(),
        path: None,
        text: Rc::from(
            "modeltype A uses 'urn:annotated';
             transformation t(out o : A);
             main() {
               var s := object Sub { name := 'x' };
               log(s.oclAsType(Base).op('1'));
               log(object Base { name := 'b' }.op('2'));
               log(s.upper);
               log('broken', s.broken);
             }",
        ),
        blackbox: None,
    };
    let meta = rs.graph.meta_mut().clone();
    let c = qvto::compile_with_metamodel(main, &NoImports, meta, &ops, &ders);
    let program = c.program.expect("compiles");
    let mut vm = qvto::Vm::new(&program, &mut rs.graph, qvto::RunOptions::default());
    let out = vm.run_transformation(vec![Vec::new()]);
    assert_eq!(out.severity, 0, "{}", out.message);
    assert_eq!(
        vm.log,
        "Sub::op 1 x\nBase::op 2\nX\nbroken, data: <Invalid>\n"
    );
}

#[test]
fn intermediate_classes_have_emf_semantics() {
    // intermediate objects are store objects (containment, one-sided opposites, container
    // references, deepclone), with static and OCL-typed features kept by the engine; the
    // expected log is Eclipse's
    let log = log_of(
        r#"modeltype Ecore uses 'http://www.eclipse.org/emf/2002/Ecore';
transformation im2(out m : Ecore);
intermediate class Node {
  name : String;
  composes kids : Node[*] opposites parent;
  references parent : Node;
  references peer : Node opposites peer2;
  references peer2 : Node;
  static count : Integer = 7;
  tags : Sequence(String);
}
main() {
  var a := object Node { name := 'a' };
  var b := object Node { name := 'b' };
  a.kids += b;
  log('parent', b.parent.name);
  log('kids', a.kids->collect(name));
  a.peer := b;
  log('peer2', b.peer2.name);
  log('count', a.count);
  a.count := 3;
  log('count b', b.count);
  var c := object Node { name := 'c' };
  log('count c', c.count);
  b.tags += 'x'; b.tags += 'y';
  log('tags', b.tags);
  var d := b.deepclone().oclAsType(Node);
  log('clone', Sequence{d.name, d.tags->size().toString()});
  log('clone parent', d.parent);
  b.parent := c;
  log('moved', c.kids->size(), a.kids->size());
  log('list', Sequence{a, b});
}
"#,
    );
    assert_eq!(
        log,
        "parent, data: a\nkids, data: {b=1}\npeer2, data: a\ncount, data: 7\ncount b, data: 3\n\
         count c, data: 3\ntags, data: [x, y]\nclone, data: [b, 2]\nclone parent, data: Node\n\
         Level 1 - moved, data: 0\nlist, data: [Node, Node]\n"
    );
}

#[test]
fn incremental_update_reuses_recorded_results() {
    // bug463572: a mapping call without a record in the run's trace starts with the result of
    // the input trace's record for the same mapping and source
    use qvto_model::{Datum, ModelStore};
    use qvto_vm::{IncrementalTrace, TraceParam, TraceRecordIn, TraceValue};
    let mut rs = ResourceSet::new();
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let epackage = meta
        .class_by_name(
            meta.package_by_ns_uri("http://www.eclipse.org/emf/2002/Ecore")
                .unwrap(),
            "EPackage",
        )
        .unwrap();
    let name = meta.feature_by_name(epackage, "name").unwrap();
    let input = rs.graph.create(epackage).unwrap();
    rs.graph.set(input, name, Datum::Str("in")).unwrap();
    let old = rs.graph.create(epackage).unwrap();
    rs.graph.set(old, name, Datum::Str("old")).unwrap();
    let main = UnitSource {
        key: "mem:t".into(),
        qualified_name: "t".into(),
        file_name: "t.qvto".into(),
        path: None,
        text: Rc::from(
            "modeltype E uses 'http://www.eclipse.org/emf/2002/Ecore';
             transformation t(in i : E, out o : E);
             main() { i.rootObjects()![EPackage].map m(); }
             mapping EPackage::m() : EPackage { if (name = null) { name := 'new' }; }",
        ),
        blackbox: None,
    };
    let c = qvto::compile(main, &NoImports, meta);
    let program = c.program.expect("compiles");
    let param = |v| TraceParam {
        name: "result".into(),
        kind: qvto_sema::ir::Direction::Out,
        type_name: "EPackage".into(),
        value: v,
    };
    let trace = IncrementalTrace {
        records: vec![TraceRecordIn {
            mapping: "m".into(),
            package: "t".into(),
            module: "t".into(),
            context: Some(TraceParam {
                name: "self".into(),
                kind: qvto_sema::ir::Direction::In,
                type_name: "EPackage".into(),
                value: TraceValue::Obj(input.0),
            }),
            params: Vec::new(),
            results: vec![param(TraceValue::Obj(old.0))],
        }],
    };
    let mut vm = qvto::Vm::new(&program, &mut rs.graph, qvto::RunOptions::default());
    vm.set_incremental_trace(trace);
    let out = vm.run_transformation(vec![vec![input], Vec::new()]);
    assert_eq!(out.severity, 0, "{}", out.message);
    let roots = out.extents[1].clone().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].index(), old.0);
    assert!(matches!(rs.graph.get(old, name), Datum::Str("old")));
}

struct OneImport(&'static str, &'static str);

impl UnitResolver for OneImport {
    fn resolve(&self, _importer: &UnitSource, q: &str) -> Option<UnitSource> {
        (q == self.0).then(|| UnitSource {
            key: format!("mem:{q}"),
            qualified_name: q.into(),
            file_name: format!("{q}.qvto"),
            path: None,
            text: Rc::from(self.1),
            blackbox: None,
        })
    }
}

#[test]
fn nested_transformation_instances_have_their_own_properties() {
    // each `new T(...)` instance keeps its property values; `t.p` reads the instance's (the
    // expected log is Eclipse's)
    let mut rs = ResourceSet::new();
    let _ = rs.ecore_resource();
    let meta = rs.graph.meta_mut().clone();
    let main = UnitSource {
        key: "mem:Outer".into(),
        qualified_name: "Outer".into(),
        file_name: "Outer.qvto".into(),
        path: None,
        text: Rc::from(
            r#"import Inner;
modeltype E uses 'http://www.eclipse.org/emf/2002/Ecore';
transformation Outer(inout m : E) access Inner;
property count : Integer = 100;
main() {
  var t1 := new Inner(m);
  var t2 := new Inner(m);
  t1.transform();
  t1.transform();
  t2.transform();
  log('outer count', count);
  log('t1 count', t1.count);
  log('t2 count', t2.count);
}
"#,
        ),
        blackbox: None,
    };
    let c = qvto::compile(
        main,
        &OneImport(
            "Inner",
            r#"modeltype E uses 'http://www.eclipse.org/emf/2002/Ecore';
transformation Inner(inout m : E);
property count : Integer = 0;
configuration property label : String;
main() {
  count := count + 1;
  log('inner main count', count);
  log('label', label);
}
query bump() : Integer { count := count + 10; return count; }
"#,
        ),
        meta,
    );
    let program = c.program.expect("compiles");
    let mut vm = qvto::Vm::new(&program, &mut rs.graph, qvto::RunOptions::default());
    let out = vm.run_transformation(vec![Vec::new()]);
    assert_eq!(out.severity, 0, "{}", out.message);
    assert_eq!(
        vm.log,
        "inner main count, data: 1\nlabel\ninner main count, data: 2\nlabel\n\
         inner main count, data: 1\nlabel\nouter count, data: 100\nt1 count, data: 2\n\
         t2 count, data: 1\n"
    );
}

#[test]
fn sorted_by_evaluates_an_effectful_body_twice() {
    // classic OCL's `evaluateSortedByIterator` runs the body over all elements twice and
    // sorts by the second values (Eclipse's log); a pure body is evaluated once
    let log = log_of(
        r#"transformation sb();
property n : Integer = 0;
helper key(x : Integer) : Integer { n := n + 1; log('key', x); return 10 - x - n; }
main() {
  var s := Sequence{3, 1, 2}->sortedBy(x | key(x));
  log('sorted', s);
  log('n', n);
  var p := Sequence{3, 1, 2}->sortedBy(x | -x);
  log('pure', p);
}
"#,
    );
    assert_eq!(
        log,
        "key, data: 3\nkey, data: 1\nkey, data: 2\nkey, data: 3\nkey, data: 1\nkey, data: 2\n\
         sorted, data: [2, 3, 1]\nn, data: 6\npure, data: [3, 2, 1]\n"
    );
}
