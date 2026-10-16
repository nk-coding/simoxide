//! Insertion-ordered hash map over values: Java's `LinkedHashMap`/`LinkedHashSet` (insertion
//! order; re-inserting a present key does not move it) and the storage of every hash-based
//! collection in fast mode.

use crate::identity::HashEnv;
use crate::java::hashmap::JavaHashMap;
use crate::value::Value;

const NIL: u32 = u32::MAX;

/// See the module docs. Entries live in a [`JavaHashMap`]; `order` lists their node ids in
/// insertion order, with holes for removed entries (compacted when half are holes).
#[derive(Clone)]
pub struct LinkedMap<V> {
    map: JavaHashMap<Value, (u32, V)>,
    order: Vec<u32>,
    holes: usize,
}

impl<V> Default for LinkedMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> LinkedMap<V> {
    pub fn new() -> Self {
        LinkedMap { map: JavaHashMap::new(), order: Vec::new(), holes: 0 }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn iter(&self) -> Iter<'_, V> {
        Iter { map: &self.map, order: self.order.iter(), left: self.map.len() }
    }
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.holes = 0;
    }

    fn compact(&mut self) {
        let mut j = 0usize;
        for i in 0..self.order.len() {
            let id = self.order[i];
            if id != NIL {
                self.order[j] = id;
                self.map.node_value_mut(id).0 = j as u32;
                j += 1;
            }
        }
        self.order.truncate(j);
        self.holes = 0;
    }

    pub fn get(&self, k: &Value, env: &dyn HashEnv) -> Option<&V> {
        self.map.get(k, env).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, k: &Value, env: &dyn HashEnv) -> Option<&mut V> {
        self.map.get_mut(k, env).map(|(_, v)| v)
    }

    /// Inserts a new key at the end: `Ok(())`, or `Err(v)` if the key is present.
    pub fn insert(&mut self, k: Value, v: V, env: &dyn HashEnv) -> Result<(), V> {
        let h = JavaHashMap::<Value, (u32, V)>::hash_of(&k, env);
        let pos = self.order.len() as u32;
        match self.map.insert_node(h, k, (pos, v), env) {
            Ok(id) => {
                self.order.push(id);
                Ok(())
            }
            Err((_, (_, v))) => Err(v),
        }
    }

    /// `put`: a present key keeps its position and gets the new value.
    pub fn put(&mut self, k: Value, v: V, env: &dyn HashEnv) -> Option<V> {
        match self.map.get_mut(&k, env) {
            Some(slot) => Some(std::mem::replace(&mut slot.1, v)),
            None => {
                let _ = self.insert(k, v, env);
                None
            }
        }
    }

    pub fn remove(&mut self, k: &Value, env: &dyn HashEnv) -> Option<(Value, V)> {
        let id = self.map.find_node(k, env)?;
        Some(self.remove_id(id, env))
    }

    fn remove_id(&mut self, id: u32, env: &dyn HashEnv) -> (Value, V) {
        let pos = self.map.node_value(id).0 as usize;
        self.order[pos] = NIL;
        self.holes += 1;
        let (k, (_, v)) = self.map.remove_node(id, true, env);
        if self.holes > 16 && self.holes * 2 > self.order.len() {
            self.compact();
        }
        (k, v)
    }

    /// Removes every entry for which `f` is false.
    pub fn retain(&mut self, mut f: impl FnMut(&Value, &V) -> bool, env: &dyn HashEnv) -> bool {
        let ids: Vec<u32> = self.order.iter().copied().filter(|&i| i != NIL).collect();
        let mut changed = false;
        for id in ids {
            if !f(self.map.node_key(id), &self.map.node_value(id).1) {
                self.remove_id(id, env);
                changed = true;
            }
        }
        changed
    }
}

/// Entries in insertion order.
pub struct Iter<'a, V> {
    map: &'a JavaHashMap<Value, (u32, V)>,
    order: std::slice::Iter<'a, u32>,
    left: usize,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a Value, &'a V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &id = self.order.next()?;
            if id != NIL {
                self.left -= 1;
                return Some((self.map.node_key(id), &self.map.node_value(id).1));
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}
