//! Stand-in for `simoxide_jcompat_exact::hashmap` in builds without the feature `compat`.
//!
//! `JavaHashMap` is uninhabited here, so the collections' Java-ordered storage (`Index::Hash`)
//! compiles to nothing and no code of the opt-in crate is linked. No value of it can exist:
//! only compat mode creates one, and compat mode is not available in such builds (a run asking
//! for it fails with "mode unavailable" before any collection is made).

use std::marker::PhantomData;

const NO_COMPAT: &str =
    "Java HashMap order needs the cargo feature `compat` (no compat environment exists without it)";

#[derive(Clone, Copy)]
enum Void {}

/// Uninhabited: see the module docs.
pub struct JavaHashMap<K, V>(Void, PhantomData<(K, V)>);

impl<K, V> Clone for JavaHashMap<K, V> {
    fn clone(&self) -> Self {
        match self.0 {}
    }
}

/// Iterator over the uninhabited map.
pub struct Iter<'a, K, V>(Void, PhantomData<&'a (K, V)>);

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {}
    }
}

/// `HashMap.tableSizeFor` (only reached through an uninhabited map).
pub fn table_size_for(_cap: usize) -> usize {
    unreachable!("{NO_COMPAT}")
}

/// `HashMap.calculateHashMapCapacity` (only reached through an uninhabited map).
pub fn hash_map_capacity_for(_n: usize) -> usize {
    unreachable!("{NO_COMPAT}")
}

impl<K, V> JavaHashMap<K, V> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        unreachable!("{NO_COMPAT}")
    }
    pub fn new_hash_map(_n: usize) -> Self {
        unreachable!("{NO_COMPAT}")
    }
    pub fn hash_of<E: ?Sized>(_key: &K, _env: &E) -> i32 {
        unreachable!("{NO_COMPAT}")
    }
    pub fn len(&self) -> usize {
        match self.0 {}
    }
    pub fn is_empty(&self) -> bool {
        match self.0 {}
    }
    pub fn capacity(&self) -> usize {
        match self.0 {}
    }
    pub fn bins_shorter_than(&self, _n: usize) -> bool {
        match self.0 {}
    }
    pub fn has_tree_bins(&self) -> bool {
        match self.0 {}
    }
    pub fn iter(&self) -> Iter<'_, K, V> {
        match self.0 {}
    }
    pub fn clear(&mut self) {
        match self.0 {}
    }
    pub fn get<E: ?Sized>(&self, _k: &K, _env: &E) -> Option<&V> {
        match self.0 {}
    }
    pub fn get_mut<E: ?Sized>(&mut self, _k: &K, _env: &E) -> Option<&mut V> {
        match self.0 {}
    }
    pub fn put<E: ?Sized>(&mut self, _k: K, _v: V, _env: &E) -> Option<V> {
        match self.0 {}
    }
    pub fn insert_node<E: ?Sized>(
        &mut self,
        _hash: i32,
        _k: K,
        _v: V,
        _env: &E,
    ) -> Result<u32, (u32, V)> {
        match self.0 {}
    }
    pub fn remove<E: ?Sized>(&mut self, _k: &K, _env: &E) -> Option<(K, V)> {
        match self.0 {}
    }
    pub fn retain<E: ?Sized>(&mut self, _f: impl FnMut(&K, &V) -> bool, _env: &E) -> bool {
        match self.0 {}
    }
    pub fn put_all<E: ?Sized, I: IntoIterator<Item = (K, V)>>(
        &mut self,
        _entries: I,
        _count: usize,
        _env: &E,
    ) {
        match self.0 {}
    }
}
