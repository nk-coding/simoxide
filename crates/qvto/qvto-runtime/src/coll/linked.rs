//! Insertion-ordered hash map over values: Java's `LinkedHashMap`/`LinkedHashSet` (insertion
//! order; re-inserting a present key does not move it) and the storage of every hash-based
//! collection in fast mode.
//!
//! Keys compare with Java `equals` and are hashed with Java `hashCode` ([`java_hash`]), which
//! also draws identity hashes exactly when Java's `LinkedHashMap` would (every `put`, `get`,
//! `remove` hashes its key). Java's bucket structure is not observable through a linked map, so
//! the table is a SwissTable (`hashbrown::HashTable`) of indices into the entries, which are
//! kept in insertion order with holes for removed entries (compacted when half are holes).

use crate::eq::{java_equals, java_hash};
use crate::identity::HashEnv;
use crate::value::Value;
use hashbrown::HashTable;

/// An entry: key, value and the (mixed) hash.
#[derive(Clone)]
struct Entry<V> {
    key: Value,
    val: V,
    hash: u64,
}

/// See the module docs.
#[derive(Clone)]
pub struct LinkedMap<V> {
    table: HashTable<u32>,
    entries: Vec<Option<Entry<V>>>,
    len: usize,
}

impl<V> Default for LinkedMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// The table hash of a key: its Java hash, mixed so that SwissTable's tag bits vary.
#[inline]
fn hash_of(k: &Value, env: &dyn HashEnv) -> u64 {
    let x = (java_hash(k, env) as u32 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^ (x >> 29)
}

impl<V> LinkedMap<V> {
    pub fn new() -> Self {
        LinkedMap {
            table: HashTable::new(),
            entries: Vec::new(),
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn iter(&self) -> Iter<'_, V> {
        Iter {
            entries: self.entries.iter(),
            left: self.len,
        }
    }
    pub fn clear(&mut self) {
        self.table.clear();
        self.entries.clear();
        self.len = 0;
    }

    #[inline]
    fn find(&self, k: &Value, hash: u64, env: &dyn HashEnv) -> Option<usize> {
        let entries = &self.entries;
        self.table
            .find(hash, |&i| {
                entries[i as usize]
                    .as_ref()
                    .is_some_and(|e| e.hash == hash && java_equals(&e.key, k, env))
            })
            .map(|&i| i as usize)
    }

    pub fn get(&self, k: &Value, env: &dyn HashEnv) -> Option<&V> {
        let h = hash_of(k, env);
        let i = self.find(k, h, env)?;
        self.entries[i].as_ref().map(|e| &e.val)
    }
    pub fn get_mut(&mut self, k: &Value, env: &dyn HashEnv) -> Option<&mut V> {
        let h = hash_of(k, env);
        let i = self.find(k, h, env)?;
        self.entries[i].as_mut().map(|e| &mut e.val)
    }

    /// Inserts a new key at the end: `Ok(())`, or `Err(v)` if the key is present.
    pub fn insert(&mut self, k: Value, v: V, env: &dyn HashEnv) -> Result<(), V> {
        let h = hash_of(&k, env);
        if self.find(&k, h, env).is_some() {
            return Err(v);
        }
        self.push_new(k, v, h);
        Ok(())
    }

    fn push_new(&mut self, k: Value, v: V, h: u64) {
        let i = self.entries.len() as u32;
        self.entries.push(Some(Entry {
            key: k,
            val: v,
            hash: h,
        }));
        let entries = &self.entries;
        self.table.insert_unique(h, i, |&j| {
            entries[j as usize].as_ref().map_or(0, |e| e.hash)
        });
        self.len += 1;
    }

    /// `put`: a present key keeps its position (and key) and gets the new value.
    pub fn put(&mut self, k: Value, v: V, env: &dyn HashEnv) -> Option<V> {
        let h = hash_of(&k, env);
        match self.find(&k, h, env) {
            Some(i) => self.entries[i]
                .as_mut()
                .map(|e| std::mem::replace(&mut e.val, v)),
            None => {
                self.push_new(k, v, h);
                None
            }
        }
    }

    pub fn remove(&mut self, k: &Value, env: &dyn HashEnv) -> Option<(Value, V)> {
        let h = hash_of(k, env);
        let i = self.find(k, h, env)?;
        Some(self.remove_index(i))
    }

    /// Removes the entry at `i` (its hash is stored: no new identity hashes are drawn).
    fn remove_index(&mut self, i: usize) -> (Value, V) {
        let h = self.entries[i].as_ref().map_or(0, |e| e.hash);
        if let Ok(occ) = self.table.find_entry(h, |&j| j as usize == i) {
            occ.remove();
        }
        let e = self.entries[i].take().expect("entry");
        self.len -= 1;
        let holes = self.entries.len() - self.len;
        if holes > 16 && holes * 2 > self.entries.len() {
            self.compact();
        }
        (e.key, e.val)
    }

    fn compact(&mut self) {
        self.entries.retain(|e| e.is_some());
        self.table.clear();
        let entries = &self.entries;
        for (i, e) in entries.iter().enumerate() {
            let h = e.as_ref().map_or(0, |e| e.hash);
            self.table.insert_unique(h, i as u32, |&j| {
                entries[j as usize].as_ref().map_or(0, |e| e.hash)
            });
        }
    }

    /// The next key in insertion order from the raw position `pos` (holes skipped), advancing
    /// `pos` past it: iteration without borrowing an iterator.
    #[inline]
    pub fn next_from(&self, pos: &mut usize) -> Option<&Value> {
        while let Some(e) = self.entries.get(*pos) {
            *pos += 1;
            if let Some(e) = e {
                return Some(&e.key);
            }
        }
        None
    }

    /// Removes every entry for which `f` is false.
    pub fn retain(&mut self, mut f: impl FnMut(&Value, &V) -> bool, _env: &dyn HashEnv) -> bool {
        let mut gone = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(e) = e
                && !f(&e.key, &e.val)
            {
                gone.push(i);
            }
        }
        if gone.is_empty() {
            return false;
        }
        for &i in &gone {
            let h = self.entries[i].as_ref().map_or(0, |e| e.hash);
            if let Ok(occ) = self.table.find_entry(h, |&j| j as usize == i) {
                occ.remove();
            }
            self.entries[i] = None;
            self.len -= 1;
        }
        let holes = self.entries.len() - self.len;
        if holes > 16 && holes * 2 > self.entries.len() {
            self.compact();
        }
        true
    }
}

/// Entries in insertion order.
pub struct Iter<'a, V> {
    entries: std::slice::Iter<'a, Option<Entry<V>>>,
    left: usize,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a Value, &'a V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.entries.next()? {
                Some(e) => {
                    self.left -= 1;
                    return Some((&e.key, &e.val));
                }
                None => continue,
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}
