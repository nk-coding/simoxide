//! Fast mode keeps the values and drops Java's order: hash-based collections iterate in
//! insertion order and hold the same elements as in compat mode.

mod common;

use common::TapeEnv;
use proptest::prelude::*;
use qvto_runtime::coll::ops;
use qvto_runtime::eq::java_equals;
use qvto_runtime::identity::Mode;
use qvto_runtime::{Coll, CollKind, Str, Value};

fn val(k: u8, x: i32) -> Value {
    match k % 4 {
        0 => Value::Int(x as i64),
        1 => Value::Str(Str::new(&format!("s{x}"))),
        2 => Value::Real(x as f64 / 2.0),
        _ => Value::long(x as i64),
    }
}

fn same_elements(a: &Coll, b: &Coll, env: &TapeEnv) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|e| b.java_count(e, env) == a.java_count(e, env))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn set_and_bag(ops_list in prop::collection::vec((any::<bool>(), 0u8..4, -30i32..30), 0..200)) {
        let compat = TapeEnv::new(Mode::Compat);
        let fast = TapeEnv::new(Mode::Fast);
        for kind in [CollKind::Set, CollKind::Bag, CollKind::OrderedSet] {
            let mut c = Coll::new_of(kind, &compat);
            let mut f = Coll::new_of(kind, &fast);
            let mut order: Vec<Value> = Vec::new();
            for &(add, k, x) in &ops_list {
                let v = val(k, x);
                if add {
                    c.add(v.clone(), &compat);
                    f.add(v.clone(), &fast);
                    if !order.iter().any(|e| java_equals(e, &v, &fast)) {
                        order.push(v);
                    }
                } else {
                    c.remove(&v, &compat);
                    f.remove(&v, &fast);
                    order.retain(|e| !java_equals(e, &v, &fast));
                }
            }
            prop_assert!(same_elements(&c, &f, &compat));
            // insertion order of the distinct elements
            let mut distinct: Vec<Value> = Vec::new();
            for e in f.iter() {
                if !distinct.iter().any(|d| java_equals(d, e, &fast)) {
                    distinct.push(e.clone());
                }
            }
            prop_assert_eq!(distinct.len(), order.len());
            for (a, b) in distinct.iter().zip(order.iter()) {
                prop_assert!(java_equals(a, b, &fast));
            }
            // operations agree up to order
            let cv: Value = c.clone().into();
            let fv: Value = f.clone().into();
            let cu = ops::qvto_including(cv.clone(), Value::Int(99), &compat);
            let fu = ops::qvto_including(fv.clone(), Value::Int(99), &fast);
            prop_assert!(same_elements(cu.as_coll().unwrap(), fu.as_coll().unwrap(), &compat));
            prop_assert_eq!(cu.as_coll().unwrap().kind(), fu.as_coll().unwrap().kind());
        }
    }
}
