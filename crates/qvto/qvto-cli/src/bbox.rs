//! The Java blackboxes of the Eclipse test bundle (`org.eclipse.m2m.tests.qvt.oml`, its buddy
//! bundle and the parser test data sources) and the reference runner's Palladio stubs, ported
//! to native implementations for the conformance adapter. Their signatures come from
//! `qvto-types/data/blackboxes.tsv`; QVTo's own libraries are `qvto::blackbox::stdlib`.
//!
//! Ported from the Eclipse QVTo test bundle (EPL-2.0): `bbox/AnnotatedJavaLibrary.java`,
//! `bbox/SimpleJavaLibrary.java`, `TestBlackboxLibrary.java`, `bbox/Bug289982_Library.java`,
//! `bbox/Bug425066_Library.java`, `bbox/Bug427237_Library.java`,
//! `bbox/CollectionTypeLibrary.java`, `bbox/StandaloneBBoxModuleRegistered.java`,
//! `buddy/bbox/StandaloneBBoxModuleUnregistered.java` and the small `Bug*_Library` classes.

use qvto::blackbox::java;
use qvto::blackbox::{NativeCall, NativeError, NativeResult, Natives};
use qvto_emf::Graph;
use qvto_runtime::Value;
use qvto_runtime::coll::CollKind;
use qvto_vm::Vm;
use std::collections::HashMap;

type V<'a, 'b> = &'a mut Vm<'b, Graph>;

const ECORE: &str = "http://www.eclipse.org/emf/2002/Ecore";
const GENERICS: &str = "http://www.eclipse.org/m2m/qvt/oml/generics";
const SIMPLEUML: &str = "http://www.eclipse.org/qvt/1.0.0/Operational/examples/simpleuml";
const BBOX: &str = "org.eclipse.m2m.tests.qvt.oml.bbox";
const BLACKBOX_TEXT: &str = "blackbox";

/// Every implementation the conformance cases need: QVTo's libraries and the test bundle's.
pub fn natives() -> Natives<Graph> {
    let mut n = Natives::new();
    qvto::blackbox::stdlib::register(&mut n);
    annotated(&mut n);
    simple(&mut n);
    test_blackbox(&mut n);
    bug289982(&mut n);
    misc(&mut n);
    palladio_stubs(&mut n);
    n
}

fn create(vm: V, ns: &str, class: &str) -> NativeResult {
    let c = vm
        .native_class_by_name(ns, class)
        .ok_or_else(|| NativeError::exception("java.lang.IllegalArgumentException", class))?;
    vm.native_create(c)
}

fn create_named(vm: V, ns: &str, class: &str, name: &str) -> NativeResult {
    let o = create(vm, ns, class)?;
    vm.native_set(&o, "name", Value::str(name))?;
    Ok(o)
}

/// Java's `String.valueOf` of an argument in a string concatenation.
fn jstr(vm: V, v: &Value) -> String {
    vm.native_to_string(v)
}

fn name_of(vm: V, o: &Value) -> Result<String, NativeError> {
    let n = vm.native_get(o, "name")?;
    Ok(jstr(vm, &n))
}

fn arg0(c: &NativeCall) -> Value {
    c.args.first().cloned().unwrap_or(Value::Null)
}

fn primitive_args(vm: V, c: &NativeCall) -> NativeResult {
    let mut items = c.args.clone();
    // `Double realArg`: `NumberConversions.convertNumber` of an integer
    if let Some(r) = items.get(2).and_then(java::real) {
        items[2] = Value::Real(r);
    }
    Ok(vm.native_collection(CollKind::Sequence, items))
}

fn first_element(vm: V, coll: &Value) -> NativeResult {
    vm.native_items(coll)
        .into_iter()
        .next()
        .ok_or_else(|| NativeError::Exception {
            class: "java.util.NoSuchElementException".into(),
            message: None,
        })
}

/// A `java.util.Date` is kept as its text; dates compare by their numeric fields.
fn date_fields(s: &str) -> Vec<i64> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .take(7)
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

fn date_before(a: &Value, b: &Value) -> NativeResult {
    let (Some(a), Some(b)) = (java::string(a), java::string(b)) else {
        return Err(NativeError::npe());
    };
    Ok(Value::Bool(date_fields(&a) < date_fields(&b)))
}

#[derive(Default)]
struct AnnotatedState {
    counters: HashMap<String, i64>,
    lib_state: Option<String>,
}

fn annotated(n: &mut Natives<Graph>) {
    let a = format!("{BBOX}.AnnotatedJavaLibrary");
    let m = |sig: &str| sig.replace("ANN", &a);
    n.java(
        &m("public java.lang.Object ANN.genericListElementAt(org.eclipse.m2m.qvt.oml.util.MutableList,int)"),
        |vm, c| {
            let items = vm.native_items(&c.self_);
            let i = java::int(&arg0(&c)).ok_or_else(NativeError::npe)?;
            items
                .get((i - 1).max(-1) as usize)
                .filter(|_| i >= 1)
                .cloned()
                .ok_or_else(|| {
                    NativeError::exception(
                        "java.lang.IndexOutOfBoundsException",
                        format!("Index {} out of bounds for length {}", i - 1, items.len()),
                    )
                })
        },
    );
    n.java(
        &m("public org.eclipse.m2m.qvt.oml.util.MutableList ANN.newListOfBooleanSetsFromBooleanCollection(java.util.Collection)"),
        |vm, c| {
            let items = vm.native_items(&arg0(&c));
            let set = vm.native_collection(CollKind::Set, items);
            Ok(vm.native_list(vec![set]))
        },
    );
    n.java(
        &m("public org.eclipse.m2m.qvt.oml.util.MutableList ANN.newList(java.util.Collection)"),
        |vm, c| {
            let items = vm.native_items(&arg0(&c));
            Ok(vm.native_list(items))
        },
    );
    n.java(
        &m("public org.eclipse.m2m.qvt.oml.util.MutableList ANN.echoList(org.eclipse.m2m.qvt.oml.util.MutableList)"),
        |vm, c| {
            let items = vm.native_items(&arg0(&c));
            Ok(vm.native_list(items))
        },
    );
    for name in [
        "primitiveTypeArgumentsHelper",
        "primitiveTypeArgumentsQuery",
        "primitiveTypeArgumentsOperation",
        "primitiveTypeArgumentsDefault",
    ] {
        n.java(
            &m(&format!("public java.util.ArrayList ANN.{name}(java.lang.String,java.lang.Integer,java.lang.Double,java.lang.Boolean)")),
            |vm, c| primitive_args(vm, &c),
        );
    }
    for name in [
        "primitiveTypeArgumentsHelperContextual",
        "primitiveTypeArgumentsQueryContextual",
        "primitiveTypeArgumentsOperationContextual",
    ] {
        n.java(
            &m(&format!("public static java.util.ArrayList ANN.{name}(org.eclipse.emf.ecore.EAnnotation,java.lang.String,java.lang.Integer,java.lang.Double,java.lang.Boolean)")),
            |vm, c| primitive_args(vm, &c),
        );
    }
    n.java(&m("public int ANN.getStrCounterX(java.lang.String)"), |vm, c| {
        let key = java::string(&c.self_).unwrap_or_default();
        let v = vm
            .native_state::<AnnotatedState>()
            .counters
            .get(&key)
            .copied()
            .unwrap_or(0);
        Ok(Value::Int(v))
    });
    n.java(&m("public int ANN.incStrCounterX(java.lang.String)"), |vm, c| {
        let key = java::string(&c.self_).unwrap_or_default();
        let st = vm.native_state::<AnnotatedState>();
        let e = st.counters.entry(key).or_insert(0);
        *e += 1;
        Ok(Value::Int(*e))
    });
    n.java(
        &m("public static java.lang.String ANN.testContextualSuffixedName(org.eclipse.emf.ecore.ENamedElement,java.lang.String)"),
        |vm, c| {
            let name = name_of(vm, &c.self_)?;
            let suffix = jstr(vm, &arg0(&c));
            Ok(Value::str(&format!("{name}{suffix}")))
        },
    );
    n.java(&m("public java.lang.String ANN.testEchoArgument(java.lang.String)"), |_vm, c| Ok(arg0(&c)));
    n.java(&m("public java.lang.String ANN.staticTestEchoArgument(java.lang.String)"), |_vm, c| Ok(arg0(&c)));
    n.java(
        &m("public static java.util.Set ANN.wrapInSet(java.util.ArrayList)"),
        |vm, c| Ok(vm.native_collection(CollKind::Set, vec![c.self_.clone()])),
    );
    n.java(
        &m("public static java.util.Set ANN.commonCollectionInSet(java.util.Collection)"),
        |vm, c| {
            let items = vm.native_items(&c.self_);
            let seq = vm.native_collection(CollKind::Sequence, items);
            Ok(vm.native_collection(CollKind::Set, vec![seq]))
        },
    );
    n.java(
        &m("public java.util.Set ANN.testComplexTypeCopy(java.util.Set)"),
        |vm, c| {
            let mut e = arg0(&c);
            for _ in 0..4 {
                e = first_element(vm, &e)?;
            }
            let bag = vm.native_collection(CollKind::Bag, vec![e]);
            let seq = vm.native_collection(CollKind::Sequence, vec![bag]);
            let os = vm.native_collection(CollKind::OrderedSet, vec![seq]);
            Ok(vm.native_collection(CollKind::Set, vec![os]))
        },
    );
    n.java(&m("public void ANN.noReturnType(java.lang.String)"), |_vm, _c| Ok(Value::Null));
    n.java(
        &m("public simpleuml.Model ANN.testUsedSimpleUMLModelPackage(simpleuml.Model)"),
        |_vm, c| Ok(arg0(&c)),
    );
    n.java(
        &m("public java.lang.String ANN.getEnumLiteralName(org.eclipse.m2m.internal.qvt.oml.expressions.DirectionKind)"),
        |vm, c| {
            vm.native_enum_name(&arg0(&c))
                .map(|s| Value::str(&s))
                .ok_or_else(NativeError::npe)
        },
    );
    // java.math.BigInteger / BigDecimal: OCL Integer / Real here
    n.java(&m("public java.math.BigInteger ANN.createBigInteger(java.lang.String)"), |_vm, c| big_integer(&arg0(&c)));
    n.java(
        &m("public java.math.BigInteger ANN.addBigInteger(java.math.BigInteger,java.math.BigInteger)"),
        |_vm, c| {
            let (Some(a), Some(b)) = (java::int(&c.args[0]), java::int(&c.args[1])) else {
                return Err(NativeError::npe());
            };
            Ok(Value::Int(a.wrapping_add(b)))
        },
    );
    n.java(&m("public java.math.BigDecimal ANN.createBigDecimal(java.lang.String)"), |_vm, c| {
        let s = java::string(&arg0(&c)).ok_or_else(NativeError::npe)?;
        s.trim()
            .parse::<f64>()
            .map(Value::Real)
            .map_err(|_| NativeError::exception("java.lang.NumberFormatException", s))
    });
    n.java(
        &m("public java.math.BigDecimal ANN.addBigDecimal(java.math.BigDecimal,java.math.BigDecimal)"),
        |_vm, c| {
            let (Some(a), Some(b)) = (java::real(&c.args[0]), java::real(&c.args[1])) else {
                return Err(NativeError::npe());
            };
            Ok(Value::Real(a + b))
        },
    );
    n.java(
        &m("public java.lang.Long ANN.sumLowerPrecisions(java.lang.Short,java.lang.Integer,java.lang.Long)"),
        |_vm, c| sum_ints(&c),
    );
    n.java(&m("public long ANN.sumLowerPrecisionWithoutWrappers(short,int,long)"), |_vm, c| sum_ints(&c));
    n.java(
        &m("public java.lang.Double ANN.sumLowerPrecisionDoubles(java.lang.Float,java.lang.Double)"),
        |_vm, c| sum_float_double(&c),
    );
    n.java(&m("public double ANN.sumLowerPrecisionDoublesWithoutWrappers(float,double)"), |_vm, c| sum_float_double(&c));
    n.java(
        &m("public static boolean ANN.theSameJavaBigIntegerInstance(java.math.BigInteger,java.math.BigInteger)"),
        |_vm, c| Ok(Value::Bool(java::int(&c.self_).is_some() && java::int(&c.self_) == java::int(&arg0(&c)))),
    );
    n.java(&m("public static java.lang.String ANN.stringContextualEcho(java.lang.String)"), |_vm, c| Ok(c.self_));
    for t in ["nonObjectBoolContextualEcho(boolean)", "boolContextualEcho(boolean)"] {
        n.java(&m(&format!("public static boolean ANN.{t}")), |_vm, c| Ok(c.self_));
    }
    for t in ["nonObjectIntContextualEcho(int)", "intContextualEcho(int)"] {
        n.java(&m(&format!("public static int ANN.{t}")), |_vm, c| {
            Ok(Value::Int(java::int(&c.self_).ok_or_else(NativeError::npe)? as i32 as i64))
        });
    }
    for t in ["nonObjectDoubleContextualEcho(double)", "doubleContextualEcho(double)"] {
        n.java(&m(&format!("public static double ANN.{t}")), |_vm, c| {
            Ok(Value::Real(java::real(&c.self_).ok_or_else(NativeError::npe)?))
        });
    }
    n.java(&m("public java.util.Date ANN.createDate(java.lang.String)"), |_vm, c| Ok(arg0(&c)));
    n.java(&m("public static boolean ANN.before(java.util.Date,java.util.Date)"), |_vm, c| date_before(&c.self_, &arg0(&c)));
    n.java(&m("public static int ANN.countIgnoreUndefined(java.util.Collection)"), |vm, c| {
        Ok(Value::Int(
            vm.native_items(&c.self_).iter().filter(|x| !x.is_null()).count() as i64,
        ))
    });
    n.java(
        &m("public static java.util.Collection ANN.shrinkAnotherCollectionToMySize(java.util.Collection,java.util.Collection)"),
        |vm, c| {
            let n = vm.native_items(&c.self_).len();
            let other = vm.native_items(&arg0(&c));
            if n > other.len() {
                return Err(NativeError::exception(
                    "java.lang.IndexOutOfBoundsException",
                    format!("toIndex = {n}"),
                ));
            }
            Ok(vm.native_collection(CollKind::Sequence, other[..n].to_vec()))
        },
    );
    n.java(&m("public java.lang.String ANN.foo()"), |_vm, _c| Ok(Value::Null));
    n.java(&m("public static java.lang.Object ANN.anyElement(java.util.Collection)"), |vm, c| first_element(vm, &c.self_));
    n.java(&m("public static java.util.Set ANN.echoAsSet(java.util.Collection)"), |vm, c| {
        let items = vm.native_items(&c.self_);
        Ok(vm.native_collection(CollKind::Set, items))
    });
    n.java(&m("public void ANN.setLibState(java.lang.String)"), |vm, c| {
        vm.native_state::<AnnotatedState>().lib_state = java::string(&arg0(&c));
        Ok(Value::Null)
    });
    n.java(&m("public java.lang.String ANN.getLibState()"), |vm, _c| {
        Ok(match &vm.native_state::<AnnotatedState>().lib_state {
            Some(s) => Value::str(s),
            None => Value::Null,
        })
    });
    n.java(&m("public java.lang.String ANN.appendToLibState(java.lang.String)"), |vm, c| {
        let add = jstr(vm, &c.self_);
        let st = vm.native_state::<AnnotatedState>();
        let s = format!("{}{add}", st.lib_state.as_deref().unwrap_or("null"));
        st.lib_state = Some(s.clone());
        Ok(Value::str(&s))
    });
    n.java(
        &m("public org.eclipse.m2m.qvt.oml.util.Dictionary ANN.toString2StringDict(org.eclipse.m2m.qvt.oml.util.Dictionary)"),
        |vm, c| {
            let d = vm.native_dict();
            for k in vm.native_dict_keys(&c.self_) {
                let v = vm.native_dict_get(&c.self_, &k);
                let ks = jstr(vm, &k);
                vm.native_dict_put(&d, Value::str(&ks), v);
            }
            Ok(d)
        },
    );
    n.java(
        &m("public void ANN.logToConsole(org.eclipse.m2m.qvt.oml.util.IContext,java.lang.String)"),
        |vm, c| {
            let s = jstr(vm, &arg0(&c));
            vm.native_log(&s);
            Ok(Value::Null)
        },
    );
    n.java(&m("public void ANN.cancelExecution(org.eclipse.m2m.qvt.oml.util.IContext)"), |vm, _c| {
        vm.native_cancel();
        Ok(Value::Null)
    });
    n.java(
        &m("public java.lang.String ANN.getConfigProperty(org.eclipse.m2m.qvt.oml.util.IContext,java.lang.String)"),
        |vm, c| {
            let key = jstr(vm, &c.self_);
            config_string(vm, &key)
        },
    );
    n.java(
        &m("public java.lang.String ANN.getConfigProperty2(org.eclipse.m2m.qvt.oml.util.IContext,java.lang.String,java.lang.String)"),
        |vm, c| {
            let key = format!("{}{}", jstr(vm, &c.self_), jstr(vm, &arg0(&c)));
            config_string(vm, &key)
        },
    );
    n.java(
        &m("public void ANN.createStereotypeApplication(org.eclipse.emf.ecore.ENamedElement)"),
        |vm, c| {
            let e = arg0(&c);
            if vm.native_has_resource(&e) {
                let name = name_of(vm, &e)?;
                let app = create_named(vm, ECORE, "EClass", &format!("{name} : stereotypeApplication"))?;
                vm.native_add_to_resource_of(&e, &app);
            }
            Ok(Value::Null)
        },
    );
    n.java(&m("public void ANN.testThrowRuntimeException()"), |_vm, _c| {
        Err(NativeError::exception("java.lang.RuntimeException", "Exception from bbox library"))
    });
    for name in ["getGenericClsWildcard", "getGenericClsInteger"] {
        n.java(&m(&format!("public generics.GenericCls ANN.{name}()")), |vm, _c| {
            let g = create(vm, GENERICS, "GenericCls")?;
            vm.native_add(&g, "values", Value::Int(1))?;
            Ok(g)
        });
    }
    n.java(&m("public generics.GenericCls ANN.getGenericClsType()"), |vm, _c| create(vm, GENERICS, "GenericCls"));
    n.java(&m("public java.sql.Time ANN.getTime()"), |_vm, _c| {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            % 86_400;
        Ok(Value::str(&format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        )))
    });
    n.java(
        &m("public java.lang.String ANN.notifierToString(org.eclipse.emf.common.notify.Notifier)"),
        |vm, c| {
            let o = arg0(&c);
            if o.is_null() {
                return Err(NativeError::npe());
            }
            Ok(Value::str(&jstr(vm, &o)))
        },
    );
}

/// `context.getConfigProperty(key).toString()`.
fn config_string(vm: V, key: &str) -> NativeResult {
    match vm.native_config(key) {
        Some(v) if !v.is_null() => Ok(Value::str(&jstr(vm, &v))),
        _ => Err(NativeError::npe()),
    }
}

fn big_integer(v: &Value) -> NativeResult {
    let s = java::string(v).ok_or_else(NativeError::npe)?;
    s.parse::<i64>()
        .map(Value::Int)
        .map_err(|_| NativeError::exception("java.lang.NumberFormatException", format!("For input string: \"{s}\"")))
}

fn sum_ints(c: &NativeCall) -> NativeResult {
    let mut sum = 0i64;
    for a in &c.args {
        sum = sum.wrapping_add(java::int(a).ok_or_else(NativeError::npe)?);
    }
    Ok(java::long(sum))
}

fn sum_float_double(c: &NativeCall) -> NativeResult {
    let (Some(f), Some(d)) = (java::real(&c.args[0]), java::real(&c.args[1])) else {
        return Err(NativeError::npe());
    };
    Ok(Value::Real((f as f32) as f64 + d))
}

fn simple(n: &mut Natives<Graph>) {
    let s = format!("{BBOX}.SimpleJavaLibrary");
    let m = |sig: &str| sig.replace("SJL", &s);
    n.java(&m("public java.lang.String SJL.echoFromSimpleJavaLibrary(java.lang.String)"), |_vm, c| Ok(arg0(&c)));
    n.java(&m("public java.util.Date SJL.simpleCreateDate(java.lang.String)"), |_vm, c| Ok(arg0(&c)));
    n.java(&m("public static boolean SJL.isBefore(java.util.Date,java.util.Date)"), |_vm, c| date_before(&c.self_, &arg0(&c)));
    // XMLCalendar(date, DATETIME): the date as an xsd:dateTime text
    n.java(
        &m("public static javax.xml.datatype.XMLGregorianCalendar SJL.toXMLCalendar(java.util.Date)"),
        |_vm, c| Ok(c.self_),
    );
    n.java(
        &m("public static java.util.Date SJL.toDate(javax.xml.datatype.XMLGregorianCalendar)"),
        |_vm, c| Ok(c.self_),
    );
    n.java(&m("public java.math.BigInteger SJL.simpleCreateBigInt(java.lang.String)"), |_vm, c| big_integer(&arg0(&c)));
}

fn test_blackbox(n: &mut Natives<Graph>) {
    n.java(
        "public java.util.Set org.eclipse.m2m.tests.qvt.oml.TestBlackboxLibrary.legacyAsRenamedSet(java.lang.Object,java.lang.String)",
        |vm, c| {
            let o = arg0(&c);
            if vm.native_instance_of(&o, ECORE, "ENamedElement") {
                vm.native_set(&o, "name", c.args[1].clone())?;
                return Ok(vm.native_collection(CollKind::Set, vec![o]));
            }
            Ok(vm.native_collection(CollKind::Set, Vec::new()))
        },
    );
    n.java(
        "public final java.util.Set org.eclipse.m2m.tests.qvt.oml.TestBlackboxLibrary.legacyOclAnyAsSet(java.lang.Object)",
        |vm, c| Ok(vm.native_collection(CollKind::Set, vec![c.self_.clone()])),
    );
}

fn log_blackbox(vm: V) {
    vm.native_log(BLACKBOX_TEXT);
}

fn set_blackbox_name(vm: V, o: &Value) -> Result<(), NativeError> {
    if o.is_null() {
        return Err(NativeError::npe());
    }
    vm.native_set(o, "name", Value::str(BLACKBOX_TEXT))
}

fn list_as_set(vm: V, l: &Value) -> NativeResult {
    if l.is_null() {
        return Err(NativeError::npe());
    }
    let items = vm.native_items(l);
    Ok(vm.native_collection(CollKind::Set, items))
}

fn dict_keys_seq(vm: V, d: &Value) -> NativeResult {
    if d.is_null() {
        return Err(NativeError::npe());
    }
    let keys = vm.native_dict_keys(d);
    Ok(vm.native_collection(CollKind::Sequence, keys))
}

fn bug289982(n: &mut Natives<Graph>) {
    let lib = format!("{BBOX}.Bug289982_Library");
    let m = |sig: &str| {
        sig.replace("DICT", "org.eclipse.m2m.qvt.oml.util.Dictionary")
            .replace("IC", "org.eclipse.m2m.qvt.oml.util.IContext")
            .replace("ML", "org.eclipse.m2m.qvt.oml.util.MutableList")
            .replace("E.", "org.eclipse.emf.ecore.")
            .replace("LIB", &lib)
    };
    // constructors
    n.java(&m("public void LIB.EPackage(E.EPackage)"), |vm, c| {
        set_blackbox_name(vm, &c.self_)?;
        Ok(Value::Null)
    });
    n.java(&m("public void LIB.EPackage(E.EPackage,ML)"), |vm, c| {
        let l = arg0(&c);
        if l.is_null() {
            return Err(NativeError::npe());
        }
        vm.native_list_add(&l, Value::str(BLACKBOX_TEXT));
        set_blackbox_name(vm, &c.self_)?;
        Ok(Value::Null)
    });
    n.java(&m("public void LIB.EModelElement(E.EModelElement)"), |vm, c| {
        let a = create(vm, ECORE, "EAnnotation")?;
        vm.native_set(&a, "source", Value::str(BLACKBOX_TEXT))?;
        vm.native_add(&c.self_, "eAnnotations", a)?;
        Ok(Value::Null)
    });
    // helpers
    n.java(&m("public void LIB.contextualHelper(E.EPackage)"), |vm, c| {
        set_blackbox_name(vm, &c.self_)?;
        Ok(Value::Null)
    });
    n.java(&m("public void LIB.nonContextualHelper()"), |_vm, _c| Ok(Value::Null));
    n.java(&m("public java.util.Set LIB.contextualHelper(E.EPackage,ML)"), |vm, c| {
        set_blackbox_name(vm, &c.self_)?;
        list_as_set(vm, &arg0(&c))
    });
    n.java(&m("public java.util.Set LIB.nonContextualHelper(ML)"), |vm, c| list_as_set(vm, &arg0(&c)));
    n.java(&m("public void LIB.contextualHelperWithExecContext(IC,E.EPackage)"), |vm, c| {
        log_blackbox(vm);
        set_blackbox_name(vm, &c.self_)?;
        Ok(Value::Null)
    });
    n.java(&m("public void LIB.nonContextualHelperWithExecContext(IC)"), |vm, _c| {
        log_blackbox(vm);
        Ok(Value::Null)
    });
    n.java(&m("public java.util.Set LIB.contextualHelperWithExecContext(IC,E.EPackage,ML)"), |vm, c| {
        log_blackbox(vm);
        set_blackbox_name(vm, &c.self_)?;
        list_as_set(vm, &arg0(&c))
    });
    n.java(&m("public java.util.Set LIB.nonContextualHelperWithExecContext(IC,ML)"), |vm, c| {
        log_blackbox(vm);
        list_as_set(vm, &arg0(&c))
    });
    // queries and operations (same bodies)
    for kind in ["Query", "Operation"] {
        n.java(&m(&format!("public void LIB.contextual{kind}(E.EClass)")), |vm, c| {
            set_blackbox_name(vm, &c.self_)?;
            Ok(Value::Null)
        });
        n.java(&m(&format!("public void LIB.nonContextual{kind}()")), |_vm, _c| Ok(Value::Null));
        n.java(&m(&format!("public java.util.List LIB.contextual{kind}(E.EClass,DICT)")), |vm, c| {
            set_blackbox_name(vm, &c.self_)?;
            dict_keys_seq(vm, &arg0(&c))
        });
        n.java(&m(&format!("public java.util.List LIB.nonContextual{kind}(DICT)")), |vm, c| dict_keys_seq(vm, &arg0(&c)));
        n.java(&m(&format!("public void LIB.contextual{kind}WithExecContext(IC,E.EClass)")), |vm, c| {
            log_blackbox(vm);
            set_blackbox_name(vm, &c.self_)?;
            Ok(Value::Null)
        });
        n.java(&m(&format!("public void LIB.nonContextual{kind}WithExecContext(IC)")), |vm, _c| {
            log_blackbox(vm);
            Ok(Value::Null)
        });
        n.java(&m(&format!("public java.util.List LIB.contextual{kind}WithExecContext(IC,E.EClass,DICT)")), |vm, c| {
            log_blackbox(vm);
            set_blackbox_name(vm, &c.self_)?;
            dict_keys_seq(vm, &arg0(&c))
        });
        n.java(&m(&format!("public java.util.List LIB.nonContextual{kind}WithExecContext(IC,DICT)")), |vm, c| {
            log_blackbox(vm);
            dict_keys_seq(vm, &arg0(&c))
        });
    }
    // mappings
    n.java(&m("public E.EClassifier LIB.contextualMapping(E.EClassifier)"), |vm, c| {
        let name = vm.native_get(&c.self_, "name")?;
        let cls = create(vm, ECORE, "EClass")?;
        vm.native_set(&cls, "name", name)?;
        Ok(cls)
    });
    n.java(&m("public E.EClassifier LIB.nonContextualMapping()"), |vm, _c| create_named(vm, ECORE, "EClass", BLACKBOX_TEXT));
    n.java(
        &m("public java.util.List LIB.contextualMapping(E.EClassifier,java.lang.Double,java.util.Collection,E.EModelElement)"),
        |vm, c| {
            let size = vm.native_items(&c.args[1]).len();
            let name = name_of(vm, &c.self_)?;
            let cls = create_named(vm, ECORE, "EClass", &format!("{name}{size}"))?;
            rename_with_number(vm, &c.args[2], &c.args[0])?;
            Ok(vm.native_collection(CollKind::Sequence, vec![cls]))
        },
    );
    n.java(
        &m("public java.util.List LIB.nonContextualMapping(java.lang.Double,java.util.Collection,E.EModelElement)"),
        |vm, c| {
            let size = vm.native_items(&c.args[1]).len();
            let cls = create_named(vm, ECORE, "EClass", &format!("{BLACKBOX_TEXT}{size}"))?;
            rename_with_number(vm, &c.args[2], &c.args[0])?;
            Ok(vm.native_collection(CollKind::Sequence, vec![cls]))
        },
    );
    n.java(&m("public E.EClassifier LIB.contextualMappingWithExecContext(IC,E.EClassifier)"), |vm, c| {
        log_blackbox(vm);
        let name = vm.native_get(&c.self_, "name")?;
        let cls = create(vm, ECORE, "EClass")?;
        vm.native_set(&cls, "name", name)?;
        Ok(cls)
    });
    n.java(&m("public E.EClassifier LIB.nonContextualMappingWithExecContext(IC)"), |vm, _c| {
        log_blackbox(vm);
        create_named(vm, ECORE, "EClass", BLACKBOX_TEXT)
    });
    n.java(&m("public java.util.List LIB.contextualMappingWithExecContext(IC,E.EClassifier,DICT)"), |vm, c| {
        log_blackbox(vm);
        let size = vm.native_dict_keys(&arg0(&c)).len();
        let name = name_of(vm, &c.self_)?;
        let cls = create_named(vm, ECORE, "EClass", &format!("{name}{size}"))?;
        Ok(vm.native_collection(CollKind::Sequence, vec![cls]))
    });
    n.java(&m("public java.util.List LIB.nonContextualMappingWithExecContext(IC,DICT)"), |vm, c| {
        log_blackbox(vm);
        let size = vm.native_dict_keys(&arg0(&c)).len();
        let cls = create_named(vm, ECORE, "EClass", &format!("{BLACKBOX_TEXT}{size}"))?;
        Ok(vm.native_collection(CollKind::Sequence, vec![cls]))
    });
    for t in ["java.util.ArrayList", "java.util.LinkedList"] {
        n.java(&m(&format!("public int LIB.ambiguousQuery({t})")), |vm, c| {
            Ok(Value::Int(vm.native_items(&arg0(&c)).len() as i64))
        });
    }
}

/// `if (e instanceof ENamedElement) e.setName(e.getName() + f.intValue())`.
fn rename_with_number(vm: V, e: &Value, f: &Value) -> Result<(), NativeError> {
    if vm.native_instance_of(e, ECORE, "ENamedElement") {
        let n = java::real(f).ok_or_else(NativeError::npe)? as i32;
        let name = name_of(vm, e)?;
        vm.native_set(e, "name", Value::str(&format!("{name}{n}")))?;
    }
    Ok(())
}

fn misc(n: &mut Natives<Graph>) {
    // Bug425066_Library
    let b = format!("{BBOX}.Bug425066_Library");
    for name in ["getEClass", "createEClass1", "createEClass2"] {
        n.java(&format!("public org.eclipse.emf.ecore.EClass {b}.{name}()"), |vm, _c| create(vm, ECORE, "EClass"));
    }
    n.java(&format!("public void {b}.EClass(org.eclipse.emf.ecore.EClass,int)"), |_vm, _c| Ok(Value::Null));
    n.java(&format!("public void {b}.EClass(org.eclipse.emf.ecore.EClass,java.lang.String)"), |_vm, _c| Ok(Value::Null));
    // Bug427237_Library: blackbox transformations
    for name in ["bug427237", "bug427237_javaless"] {
        n.transformation(
            name,
            &format!("public void {BBOX}.Bug427237_Library.{name}(org.eclipse.m2m.qvt.oml.util.IContext,org.eclipse.m2m.qvt.oml.blackbox.java.JavaModelInstance,org.eclipse.m2m.qvt.oml.blackbox.java.JavaModelInstance)"),
            |vm, c| {
                let prop = vm.native_config("prop").unwrap_or(Value::Null);
                let prop = jstr(vm, &prop);
                let root = create_named(vm, ECORE, "EPackage", &format!("root{prop}"))?;
                let (input, output) = (c.args[0].clone(), c.args[1].clone());
                vm.native_model_add(&output, &root);
                for i in 0..vm.native_model_roots(&input).len() {
                    let cls = create_named(vm, ECORE, "EClass", &format!("cls{i}"))?;
                    vm.native_add(&root, "eClassifiers", cls.clone())?;
                    vm.native_model_add(&output, &cls);
                }
                Ok(Value::Null)
            },
        );
    }
    // CollectionTypeLibrary
    let ct = format!("{BBOX}.CollectionTypeLibrary");
    n.java(&format!("public java.util.Collection {ct}.commonCollectionOperation(java.util.Collection)"), |vm, _c| Ok(vm.native_list(Vec::new())));
    n.java(&format!("public java.util.Set {ct}.setOperation(java.util.Set)"), |vm, _c| Ok(vm.native_collection(CollKind::Set, Vec::new())));
    n.java(&format!("public java.lang.String {ct}.stringCollectionAny(java.util.Collection)"), |vm, c| first_element(vm, &c.self_));
    // standalone modules
    for (class, suffix) in [
        (format!("{BBOX}.StandaloneBBoxModuleRegistered"), "Registered"),
        ("org.eclipse.m2m.tests.qvt.oml.buddy.bbox.StandaloneBBoxModuleUnregistered".to_string(), "Unregistered"),
    ] {
        n.java(&format!("public java.lang.String {class}.echoFromBBoxModule{suffix}(java.lang.String)"), |_vm, c| Ok(arg0(&c)));
        n.java(
            &format!("public org.eclipse.emf.ecore.EClass {class}.modifyClassName{suffix}(org.eclipse.emf.ecore.EClass,java.lang.String)"),
            |vm, c| {
                let cls = arg0(&c);
                vm.native_set(&cls, "name", c.args[1].clone())?;
                Ok(cls)
            },
        );
    }
    // Bug565747_Library
    n.java(&format!("public java.lang.String {BBOX}.Bug565747_Library.print(org.eclipse.emf.ecore.EObject)"), |vm, c| {
        Ok(Value::str(&jstr(vm, &c.self_)))
    });
    // Bug577992_Library: `self.op(t)` throws (generated EOperation stub), then the attribute
    n.java(
        &format!("public {BBOX}.bug577992.TestClass {BBOX}.Bug577992_Library.copyName({BBOX}.bug577992.TestClass,{BBOX}.bug577992.TestClass)"),
        |vm, c| {
            let t = arg0(&c);
            let attr = vm.native_get(&t, "attr")?;
            vm.native_set(&c.self_, "attr", attr)?;
            Ok(t)
        },
    );
    n.java("public org.eclipse.emf.ecore.EClass org.eclipse.m2m.tests.qvto.transformationProject.Bug507955_Library.mirrorBug507955(org.eclipse.emf.ecore.EClass)", |_vm, c| Ok(arg0(&c)));
    n.java(&format!("public org.eclipse.emf.ecore.EClass {BBOX}.Bug579914_Library.mirrorBug579914(org.eclipse.emf.ecore.EClass)"), |_vm, c| Ok(arg0(&c)));
    n.java(&format!("public simpleuml.Class {BBOX}.Bug573752_Library.test(simpleuml.DataType)"), |vm, _c| create(vm, SIMPLEUML, "Class"));
    n.java("public java.lang.String org.eclipse.m2m.tests.qvt.oml.Bug578955_Library.echoBug578955(java.lang.String)", |_vm, c| Ok(arg0(&c)));
    n.java("public java.lang.String org.eclipse.m2m.tests.qvt.oml.Blackbox.doEchoFromSimpleJavaLibrary(java.lang.String)", |_vm, c| Ok(arg0(&c)));
    n.java("public java.lang.String org.eclipse.m2m.tests.qvt.oml.Bug578265_Library.doEchoFromSimpleJavaLibrary(java.lang.String)", |_vm, c| Ok(arg0(&c)));
}

/// The reference runner's stub of Palladio's `ProfilesLibrary` (architectural templates):
/// every operation throws (`reference/runner/src-stubs`).
fn palladio_stubs(n: &mut Natives<Graph>) {
    let p = "org.palladiosimulator.architecturaltemplates.catalog.black.ProfilesLibrary";
    for sig in [
        "public static boolean P.isProfileApplied(org.eclipse.emf.ecore.EObject,java.lang.String)",
        "public static void P.applyProfile(org.eclipse.emf.ecore.EObject,java.lang.String)",
        "public static boolean P.hasAppliedStereotype(org.eclipse.emf.ecore.EObject,java.lang.String)",
        "public static void P.applyStereotype(org.eclipse.emf.ecore.EObject,java.lang.String)",
        "public static void P.removeStereotypeApplications(org.eclipse.emf.ecore.EObject,java.lang.String)",
        "public static java.lang.Object P.getTaggedValue(org.eclipse.emf.ecore.EObject,java.lang.String,java.lang.String)",
        "public static void P.setTaggedValue(org.eclipse.emf.ecore.EObject,java.lang.Object,java.lang.String,java.lang.String)",
    ] {
        n.java(&sig.replace("P.", &format!("{p}.")), |_vm, _c| {
            Err(NativeError::exception(
                "java.lang.UnsupportedOperationException",
                "qvto-rs reference runner: ProfilesLibrary stub",
            ))
        });
    }
}
