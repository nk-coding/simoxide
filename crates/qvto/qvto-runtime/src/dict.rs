//! QVTo's mutable `Dict` (`DictionaryImpl` over a `HashMap`) and `List` (`MutableListImpl`,
//! an `ArrayList`). Both are shared by reference (`Rc<RefCell<_>>` in [`Value`]).

use crate::coll::Index;
use crate::eq::{java_equals, java_hash};
use crate::identity::HashEnv;
use crate::value::Value;

/// `DictionaryImpl`: a `HashMap` (compat: bucket order; fast: insertion order). Iterating a
/// dictionary as a collection yields its values.
#[derive(Clone)]
pub struct Dict {
    pub map: Index<Value>,
}

impl Dict {
    /// `new DictionaryImpl()`.
    pub fn new(env: &dyn HashEnv) -> Dict {
        Dict {
            map: if env.compat() {
                Index::Hash(crate::java::hashmap::JavaHashMap::new())
            } else {
                Index::Linked(crate::coll::LinkedMap::new())
            },
        }
    }

    /// `new DictionaryImpl(source)`: `new HashMap(source.fMap)` (pre-sized, filled in the
    /// source's order).
    pub fn copy(&self, env: &dyn HashEnv) -> Dict {
        match &self.map {
            Index::Hash(m) => {
                let mut n = crate::java::hashmap::JavaHashMap::new();
                n.put_all(m.iter().map(|(k, v)| (k.clone(), v.clone())), m.len(), env);
                Dict {
                    map: Index::Hash(n),
                }
            }
            Index::Linked(_) => {
                let mut d = Dict::new(env);
                for (k, v) in self.map.iter() {
                    d.map.put(k.clone(), v.clone(), env);
                }
                d
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn get(&self, k: &Value, env: &dyn HashEnv) -> Option<&Value> {
        self.map.get(k, env)
    }
    pub fn put(&mut self, k: Value, v: Value, env: &dyn HashEnv) -> Option<Value> {
        self.map.put(k, v, env)
    }
    pub fn has_key(&self, k: &Value, env: &dyn HashEnv) -> bool {
        self.map.get(k, env).is_some()
    }
    pub fn clear(&mut self) {
        self.map.clear()
    }
    /// `remove(Object)` of the `Collection` view: removes the first entry (in iteration order)
    /// whose value equals `v`, through the iterator.
    pub fn remove_value(&mut self, v: &Value, env: &dyn HashEnv) -> bool {
        let mut done = false;
        self.map.retain(
            |_, w| {
                if !done && java_equals(v, w, env) {
                    done = true;
                    false
                } else {
                    true
                }
            },
            env,
        );
        done
    }
    /// Entries in iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.map.iter()
    }
    /// `keys()`: a new list in iteration order.
    pub fn keys(&self) -> Vec<Value> {
        self.map.iter().map(|(k, _)| k.clone()).collect()
    }
    /// `values()`: a new list in iteration order.
    pub fn values(&self) -> Vec<Value> {
        self.map.iter().map(|(_, v)| v.clone()).collect()
    }

    /// `HashMap.hashCode`: sum of `hash(key) ^ hash(value)`.
    pub fn java_hash(&self, env: &dyn HashEnv) -> i32 {
        self.map.iter().fold(0i32, |h, (k, v)| {
            h.wrapping_add(java_hash(k, env) ^ java_hash(v, env))
        })
    }

    /// `AbstractMap.equals`.
    pub fn java_equals(&self, other: &Dict, env: &dyn HashEnv) -> bool {
        self.len() == other.len()
            && self.map.iter().all(|(k, v)| match other.map.get(k, env) {
                Some(w) => java_equals(v, w, env),
                None => false,
            })
    }
}

/// `MutableListImpl`.
#[derive(Clone, Default, Debug)]
pub struct List {
    pub items: Vec<Value>,
}

impl List {
    pub fn new() -> List {
        List { items: Vec::new() }
    }
    pub fn from_vec(items: Vec<Value>) -> List {
        List { items }
    }
}

impl std::fmt::Debug for Dict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Dict")?;
        f.debug_map().entries(self.iter()).finish()
    }
}
