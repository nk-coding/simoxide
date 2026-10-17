//! Native blackboxes end to end: a host library declared as signature rows and Rust closures,
//! QVTo's `Strings`/`emf.tools`, Palladio's simulation time, `blackbox` operations declared in
//! QVTo, blackbox transformations, Java exceptions and cancellation.

use qvto::blackbox::{BlackboxResolver, Catalog, NativeError, Natives, palladio, stdlib};
use qvto::{UnitResolver, UnitSource};
use qvto_emf::{Graph, ResourceSet};
use qvto_model::ModelStore;
use qvto_runtime::Value;
use std::collections::HashMap;
use std::rc::Rc;

/// Units from strings.
struct Mem(HashMap<String, String>);

fn unit(name: &str, text: &str) -> UnitSource {
    UnitSource {
        key: format!("mem:{name}"),
        qualified_name: name.to_string(),
        file_name: format!("{name}.qvto"),
        path: None,
        text: Rc::from(text),
        blackbox: None,
    }
}

impl UnitResolver for Mem {
    fn resolve(&self, _importer: &UnitSource, qualified_name: &str) -> Option<UnitSource> {
        self.0.get(qualified_name).map(|t| unit(qualified_name, t))
    }
}

const ECORE: &str = "http://www.eclipse.org/emf/2002/Ecore";

/// A host library: unit `host.Lib`, module `HostLib` using Ecore.
const HOST: &str = "\
env\thost\thost\t-\t-
unit\thost\thost.Lib\tqvto://blackbox/host.Lib\tOK
module\thost\thost.Lib\tHostLib\tLibrary\thost.Lib\thttp://www.eclipse.org/emf/2002/Ecore
op\thost\thost.Lib\tHostLib\ttwice\thelper\t-\tfalse\ts:in:oclstdlib::String\toclstdlib::String\thost#twice\t
op\thost\thost.Lib\tHostLib\tlabel\tquery\thttp://www.eclipse.org/emf/2002/Ecore#ENamedElement\tfalse\tsuffix:in:oclstdlib::String\toclstdlib::String\thost#label\t
op\thost\thost.Lib\tHostLib\tmakeClass\tmapping\t-\tfalse\tname:in:oclstdlib::String\thttp://www.eclipse.org/emf/2002/Ecore#EClass\thost#makeClass\t
op\thost\thost.Lib\tHostLib\tfail\tquery\t-\tfalse\t\toclstdlib::String\thost#fail\t
op\thost\thost.Lib\tHostLib\tstop\thelper\t-\tfalse\t\toclstdlib::OclVoid\thost#stop\twithExecutionContext
op\thost\thost.Lib\tHostLib\tcount\thelper\t-\tfalse\t\toclstdlib::Integer\thost#count\t
op\thost\thost.Lib\tHostLib\tdeclared\tquery\t-\tfalse\tx:in:oclstdlib::Integer\toclstdlib::Integer\thost#declared\t
";

#[derive(Default)]
struct Counter(i64);

fn natives() -> Natives<Graph> {
    let mut n = Natives::new();
    stdlib::register(&mut n);
    palladio::register_simulation_time(&mut n, || 42.5);
    n.op("host#twice", |_vm, c| {
        let s = qvto::blackbox::java::string(&c.args[0]).unwrap_or_default();
        Ok(Value::str(&format!("{s}{s}")))
    });
    n.op("host#label", |vm, c| {
        let name = vm.native_get(&c.self_, "name")?;
        let name = vm.native_to_string(&name);
        let suffix = vm.native_to_string(&c.args[0]);
        Ok(Value::str(&format!("{name}{suffix}")))
    });
    n.op("host#makeClass", |vm, c| {
        let cls = vm.native_class_by_name(ECORE, "EClass").unwrap();
        let o = vm.native_create(cls)?;
        vm.native_set(&o, "name", c.args[0].clone())?;
        Ok(o)
    });
    n.op("host#fail", |_vm, _c| {
        Err(NativeError::exception(
            "java.lang.IllegalStateException",
            "broken",
        ))
    });
    n.op("host#stop", |vm, _c| {
        vm.native_log("stopping");
        vm.native_cancel();
        Ok(Value::Null)
    });
    n.op("host#count", |vm, _c| {
        let st = vm.native_state::<Counter>();
        st.0 += 1;
        Ok(Value::Int(st.0))
    });
    n.op("host#declared", |_vm, c| {
        Ok(Value::Int(
            qvto::blackbox::java::int(&c.args[0]).unwrap_or(0) * 10,
        ))
    });
    n.transformation("Native", "host transformation Native", |vm, c| {
        let cls = vm.native_class_by_name(ECORE, "EPackage").unwrap();
        let p = vm.native_create(cls)?;
        let n = vm.native_model_roots(&c.args[0]).len();
        vm.native_set(&p, "name", Value::str(&format!("made from {n}")))?;
        vm.native_model_add(&c.args[1], &p);
        Ok(Value::Null)
    });
    n
}

/// Compiles `main` (with the given extra units) against Ecore, runs it without inputs, returns
/// the log, the outcome's severity/message and the output roots' names.
fn run(main: &str, units: &[(&str, &str)], registered: &[&str]) -> (String, i32, String) {
    let mut cat = stdlib::catalog();
    cat.extend(&Catalog::parse(HOST).unwrap());
    cat.extend(&palladio::catalog());
    let files = Mem(units
        .iter()
        .map(|(n, t)| (n.to_string(), t.to_string()))
        .collect());
    let mut reg = Vec::new();
    for env in registered {
        reg.extend(cat.units(env).iter().cloned());
    }
    let mut classpath = cat.units("host").to_vec();
    classpath.extend(cat.units("simulizar").iter().cloned());
    let resolver = BlackboxResolver {
        inner: &files,
        registered: reg,
        classpath,
    };
    let mut rs = ResourceSet::new();
    rs.ecore_resource();
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let c =
        qvto::compile_with_operations(unit("t", main), &resolver, rs.graph.meta().clone(), &ops);
    for u in &c.units {
        for p in &u.problems {
            eprintln!("{} {}: {}", u.source.file_name, p.line(&u.lines), p.message);
        }
    }
    let program = c.program.expect("compiles");
    let main_mod = &program.ir.modules[program.ir.main_module.index()];
    let args = vec![Vec::new(); main_mod.model_params.len()];
    let mut vm = qvto::Vm::new(&program, &mut rs.graph, qvto::RunOptions::default());
    vm.set_natives(Rc::new(natives()));
    let out = vm.run_transformation(args);
    let log = std::mem::take(&mut vm.log);
    (log, out.severity, out.message)
}

#[test]
fn host_library_operations() {
    let (log, sev, msg) = run(
        "import host.Lib;
import org.palladiosimulator.simulizar.action.repository.black.SimulationStateLibrary;
modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
transformation t(out o : E);
main() {
    log(twice('ab'));
    var c := map makeClass('C');
    log(c.name);
    log(c.label('!'));
    log(count().toString() + count().toString());
    log(getSimulationTime().toString());
}",
        &[],
        &[],
    );
    assert_eq!(sev, 0, "{msg}\n{log}");
    assert_eq!(log, "abab\nC\nC!\n12\n42.5\n");
}

#[test]
fn strings_and_emf_tools() {
    let (log, sev, msg) = run(
        "import Strings;
import emf.tools;
modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
transformation t(out o : E);
main() {
    log('a,b,,c,,'.split(',')->size().toString());
    log('a1b2'.split('[0-9]', 0)->first());
    log('abcabc'.indexOf('c', 3).toString() + ' ' + 'abcabc'.lastIndexOf('a', 10).toString());
    -- ambiguous with the standard library's (non-regex) replace, which Eclipse chooses
    log('hello'.replace('l+', 'L'));
    log('hello'.startsWith('ll', 2).toString());
    var a := object EAnnotation { source := 's'; details += object EStringToStringMapEntry { key := 'k'; value := 'v' } };
    log(a.getDetails('k'));
    log(getDataTypeInstance('http://www.eclipse.org/emf/2002/Ecore', 'EString').name);
}",
        &[],
        &["stdlib"],
    );
    assert_eq!(sev, 0, "{msg}\n{log}");
    assert_eq!(log, "4\na\n5 4\nhello\ntrue\nv\nEString\n");
}

#[test]
fn declared_blackbox_operation_and_errors() {
    let (log, sev, msg) = run(
        "modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
transformation t(out o : E);
blackbox query declared(x : Integer) : Integer;
main() {
    log(declared(4).toString());
}",
        &[],
        &["host"],
    );
    assert_eq!(sev, 0, "{msg}\n{log}");
    assert_eq!(log, "40\n");

    let (log, sev, msg) = run(
        "import host.Lib;
modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
transformation t(out o : E);
main() {
    log(fail());
}",
        &[],
        &[],
    );
    assert_eq!(sev, 4);
    assert_eq!(
        msg,
        "Java method invocation error 'host#fail'\nCaused by: java.lang.IllegalStateException: broken"
    );
    assert!(log.starts_with("Terminating execution..."), "{log}");

    let (log, sev, _) = run(
        "import host.Lib;
modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
transformation t(out o : E);
main() {
    stop();
    log('not reached');
}",
        &[],
        &[],
    );
    assert_eq!(sev, 8, "{log}");
    assert!(log.starts_with("stopping\n"), "{log}");
    assert!(!log.contains("not reached"));
}

#[test]
fn blackbox_transformation() {
    let (log, sev, msg) = run(
        "import Native;
modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
transformation t(in i : E, out o : E) access Native;
main() {
    var s := new Native(i, o).transform();
    log(s.succeeded().toString());
    log(o.rootObjects()![EPackage].name);
}",
        &[(
            "Native",
            "modeltype E uses ecore('http://www.eclipse.org/emf/2002/Ecore');
blackbox transformation Native(in a : E, out b : E);",
        )],
        &[],
    );
    assert_eq!(sev, 0, "{msg}\n{log}");
    assert_eq!(log, "true\nmade from 0\n");
}

#[test]
fn java_method_keys() {
    assert_eq!(
        qvto::blackbox::java_method_key(
            "public static java.util.Set org.x.Lib.echo(java.util.Collection,int)"
        ),
        "org.x.Lib#echo(java.util.Collection,int)"
    );
    assert_eq!(
        qvto::blackbox::java_method_key("public void a.B.c()"),
        "a.B#c()"
    );
}
