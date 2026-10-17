//! The closed-form order of insert-only maps (`order::iteration_order`) equals the iteration
//! order of the full `HashMap` reimplementation.

use simoxide_jcompat::hash::str_hash;
use simoxide_jcompat_exact::hashmap::{JavaHashMap, JavaKey};
use simoxide_jcompat_exact::order::iteration_order;
use std::cmp::Ordering;

#[derive(Clone, PartialEq, Eq, Debug)]
struct JStr(String);

impl JavaKey<()> for JStr {
    fn java_hash(&self, _: &()) -> i32 {
        str_hash(&self.0)
    }
    fn java_equals(&self, other: &Self, _: &()) -> bool {
        self.0 == other.0
    }
    fn comparable_class(&self) -> Option<u8> {
        Some(0)
    }
    fn compare_to(&self, other: &Self) -> Ordering {
        self.0.encode_utf16().cmp(other.0.encode_utf16())
    }
    fn class_name<'a>(&'a self, _: &'a ()) -> Option<&'a str> {
        Some("java.lang.String")
    }
    fn identity_hash(&self, _: &()) -> i32 {
        0
    }
}

#[test]
fn closed_form_equals_hash_map() {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    for round in 0..2000 {
        let n = (next() % 200) as usize;
        let mut keys: Vec<String> = Vec::new();
        for _ in 0..n {
            // short ids like SimOxide's (container and resource-type ids), with repeats
            let k = format!(
                "_{}{}",
                ["a", "B", "id", "x_"][(next() % 4) as usize],
                next() % 500
            );
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        let mut m = JavaHashMap::new();
        for k in &keys {
            m.put(JStr(k.clone()), (), &());
        }
        if m.has_tree_bins() {
            continue;
        }
        let hashes: Vec<i32> = keys.iter().map(|k| str_hash(k)).collect();
        let closed: Vec<&str> = iteration_order(&hashes)
            .into_iter()
            .map(|i| keys[i].as_str())
            .collect();
        let full: Vec<&str> = m.keys().map(|k| k.0.as_str()).collect();
        assert_eq!(closed, full, "round {round}");
    }
}
