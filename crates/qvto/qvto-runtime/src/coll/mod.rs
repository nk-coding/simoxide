//! OCL collections with the Java classes Eclipse uses for them.
//!
//! The runtime kind of a collection is its Java class, which can differ from its static OCL
//! type (`objectsOfType` is statically a `Set` but returns a `LinkedHashSet`; compat row 3):
//!
//! | [`CollKind`] | Java class | compat storage | fast storage |
//! |---|---|---|---|
//! | `Set` | `HashSet` | [`JavaHashMap`] (bucket order) | [`LinkedMap`] (insertion order) |
//! | `OrderedSet` | `LinkedHashSet` | [`LinkedMap`] | [`LinkedMap`] |
//! | `Bag` | `org.eclipse.ocl.util.BagImpl` | `JavaHashMap<E, count>` | `LinkedMap<E, count>` |
//! | `Sequence` | `ArrayList` | `Vec` | `Vec` |
//!
//! The mutating methods here are the Java collection methods (`add`, `remove`, `addAll`, the
//! copy constructors); OCL's value-returning operations are in [`ops`] and build new
//! collections with them, as `CollectionUtil` does.

pub mod linked;
pub mod ops;

pub use linked::LinkedMap;

use crate::eq::java_hash;
use crate::identity::HashEnv;
use crate::java::hashmap::JavaHashMap;
use crate::value::Value;
use std::cell::Cell;

/// The runtime kind (Java class) of a collection.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum CollKind {
    /// `java.util.HashSet`.
    Set,
    /// `java.util.LinkedHashSet`.
    OrderedSet,
    /// `org.eclipse.ocl.util.BagImpl`.
    Bag,
    /// `java.util.ArrayList`.
    Sequence,
}

impl CollKind {
    /// The OCL name (`CollectionKind.getName()`).
    pub fn ocl_name(self) -> &'static str {
        match self {
            CollKind::Set => "Set",
            CollKind::OrderedSet => "OrderedSet",
            CollKind::Bag => "Bag",
            CollKind::Sequence => "Sequence",
        }
    }
    /// `getClass().getName()`.
    pub fn java_class(self) -> &'static str {
        match self {
            CollKind::Set => "java.util.HashSet",
            CollKind::OrderedSet => "java.util.LinkedHashSet",
            CollKind::Bag => "org.eclipse.ocl.util.BagImpl",
            CollKind::Sequence => "java.util.ArrayList",
        }
    }
    /// `instanceof java.util.Set` (HashSet and LinkedHashSet).
    pub fn is_set(self) -> bool {
        matches!(self, CollKind::Set | CollKind::OrderedSet)
    }
}

thread_local! {
    static NEXT_CELL: Cell<u64> = const { Cell::new(0) };
}

/// A fresh identity for a `BagImpl.MutableInteger` count cell.
fn new_cell() -> u64 {
    NEXT_CELL.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        v
    })
}

/// A bag entry: the count and the identity of Java's `MutableInteger` holding it (which enters
/// `BagImpl.hashCode`).
#[derive(Clone, Copy, Debug)]
pub struct BagCell {
    pub count: u32,
    pub cell: u64,
}

/// Storage of a hash-based collection.
#[derive(Clone)]
pub enum Index<V> {
    Hash(JavaHashMap<Value, V>),
    Linked(LinkedMap<V>),
}

impl<V> Index<V> {
    fn new(env: &dyn HashEnv) -> Self {
        if env.compat() {
            Index::Hash(JavaHashMap::new())
        } else {
            Index::Linked(LinkedMap::new())
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Index::Hash(m) => m.len(),
            Index::Linked(m) => m.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn iter(&self) -> IndexIter<'_, V> {
        match self {
            Index::Hash(m) => IndexIter::Hash(m.iter()),
            Index::Linked(m) => IndexIter::Linked(m.iter()),
        }
    }
    pub fn get(&self, k: &Value, env: &dyn HashEnv) -> Option<&V> {
        match self {
            Index::Hash(m) => m.get(k, env),
            Index::Linked(m) => m.get(k, env),
        }
    }
    pub fn get_mut(&mut self, k: &Value, env: &dyn HashEnv) -> Option<&mut V> {
        match self {
            Index::Hash(m) => m.get_mut(k, env),
            Index::Linked(m) => m.get_mut(k, env),
        }
    }
    /// Inserts if absent; `false` if the key was present (nothing changes then).
    pub fn insert(&mut self, k: Value, v: V, env: &dyn HashEnv) -> bool {
        match self {
            Index::Hash(m) => {
                let h = JavaHashMap::<Value, V>::hash_of(&k, env);
                m.insert_node(h, k, v, env).is_ok()
            }
            Index::Linked(m) => m.insert(k, v, env).is_ok(),
        }
    }
    /// `put`: inserts or replaces the value.
    pub fn put(&mut self, k: Value, v: V, env: &dyn HashEnv) -> Option<V> {
        match self {
            Index::Hash(m) => m.put(k, v, env),
            Index::Linked(m) => m.put(k, v, env),
        }
    }
    pub fn remove(&mut self, k: &Value, env: &dyn HashEnv) -> Option<(Value, V)> {
        match self {
            Index::Hash(m) => m.remove(k, env),
            Index::Linked(m) => m.remove(k, env),
        }
    }
    /// Iterator removal of every entry for which `f` is false.
    pub fn retain(&mut self, f: impl FnMut(&Value, &V) -> bool, env: &dyn HashEnv) -> bool {
        match self {
            Index::Hash(m) => m.retain(f, env),
            Index::Linked(m) => m.retain(f, env),
        }
    }
    pub fn clear(&mut self) {
        match self {
            Index::Hash(m) => m.clear(),
            Index::Linked(m) => m.clear(),
        }
    }
    /// True if the storage keeps Java's bucket order.
    pub fn is_hash(&self) -> bool {
        matches!(self, Index::Hash(_))
    }
}

/// Iterator over an [`Index`].
pub enum IndexIter<'a, V> {
    Hash(crate::java::hashmap::Iter<'a, Value, V>),
    Linked(linked::Iter<'a, V>),
}

impl<'a, V> Iterator for IndexIter<'a, V> {
    type Item = (&'a Value, &'a V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            IndexIter::Hash(i) => i.next(),
            IndexIter::Linked(i) => i.next(),
        }
    }
}

/// A `BagImpl`: element counts plus the total size.
#[derive(Clone)]
pub struct Bag {
    pub index: Index<BagCell>,
    pub size: usize,
}

/// An OCL collection value.
#[derive(Clone)]
pub enum Coll {
    /// `HashSet` (compat: bucket order; fast: insertion order).
    Set(Index<()>),
    /// `LinkedHashSet`.
    OrderedSet(LinkedMap<()>),
    /// `BagImpl`.
    Bag(Bag),
    /// `ArrayList`.
    Sequence(Vec<Value>),
}

impl Coll {
    /// `new HashSet()`; in ordered mode a `LinkedHashSet` (the det JVM's rewritten
    /// allocation: also its class, which OCL tests with `instanceof LinkedHashSet` to tell an
    /// OrderedSet, so such a set behaves as an OrderedSet in equality, `asSet`, `oclIsKindOf`,
    /// printing, ...).
    pub fn new_set(env: &dyn HashEnv) -> Coll {
        if env.ordered() {
            return Coll::new_ordered_set();
        }
        Coll::Set(Index::new(env))
    }
    /// `new LinkedHashSet()`.
    pub fn new_ordered_set() -> Coll {
        Coll::OrderedSet(LinkedMap::new())
    }
    /// `new BagImpl()`.
    pub fn new_bag(env: &dyn HashEnv) -> Coll {
        Coll::Bag(Bag {
            index: Index::new(env),
            size: 0,
        })
    }
    /// `new ArrayList()`.
    pub fn new_sequence() -> Coll {
        Coll::Sequence(Vec::new())
    }
    /// `createNewCollection(kind)`.
    pub fn new_of(kind: CollKind, env: &dyn HashEnv) -> Coll {
        match kind {
            CollKind::Set => Coll::new_set(env),
            CollKind::OrderedSet => Coll::new_ordered_set(),
            CollKind::Bag => Coll::new_bag(env),
            CollKind::Sequence => Coll::new_sequence(),
        }
    }

    /// `createNewCollection(kind, c)`: the copy constructor of the kind's Java class, filled in
    /// `c`'s iteration order (`new HashSet(c)` is pre-sized as JDK 21 does).
    pub fn copy_of(kind: CollKind, c: &Coll, env: &dyn HashEnv) -> Coll {
        let mut r = match kind {
            CollKind::Set => {
                if env.compat() {
                    Coll::Set(Index::Hash(JavaHashMap::new_hash_map(c.len().max(12))))
                } else {
                    Coll::new_set(env)
                }
            }
            CollKind::Sequence => Coll::Sequence(Vec::with_capacity(c.len())),
            k => Coll::new_of(k, env),
        };
        r.add_all(c, env);
        r
    }

    /// The same as [`copy_of`](Self::copy_of) from a list of values.
    pub fn from_values<I: IntoIterator<Item = Value>>(
        kind: CollKind,
        values: I,
        env: &dyn HashEnv,
    ) -> Coll {
        let mut r = Coll::new_of(kind, env);
        for v in values {
            r.add(v, env);
        }
        r
    }

    pub fn kind(&self) -> CollKind {
        match self {
            Coll::Set(_) => CollKind::Set,
            Coll::OrderedSet(_) => CollKind::OrderedSet,
            Coll::Bag(_) => CollKind::Bag,
            Coll::Sequence(_) => CollKind::Sequence,
        }
    }

    /// `size()` (a bag counts repetitions).
    pub fn len(&self) -> usize {
        match self {
            Coll::Set(s) => s.len(),
            Coll::OrderedSet(s) => s.len(),
            Coll::Bag(b) => b.size,
            Coll::Sequence(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Elements in Java iteration order (a bag repeats each element `count` times in a row).
    pub fn iter(&self) -> CollIter<'_> {
        match self {
            Coll::Set(s) => CollIter::Set(s.iter()),
            Coll::OrderedSet(s) => CollIter::OSet(s.iter()),
            Coll::Bag(b) => CollIter::Bag {
                it: b.index.iter(),
                cur: None,
                left: 0,
            },
            Coll::Sequence(v) => CollIter::Seq(v.iter()),
        }
    }

    /// The elements as a vector, in iteration order.
    pub fn to_vec(&self) -> Vec<Value> {
        self.iter().cloned().collect()
    }

    /// Java `add`: `false` if a set already contains the element.
    pub fn add(&mut self, v: Value, env: &dyn HashEnv) -> bool {
        match self {
            Coll::Set(s) => s.insert(v, (), env),
            Coll::OrderedSet(s) => s.insert(v, (), env).is_ok(),
            Coll::Bag(b) => {
                b.size += 1;
                if let Some(c) = b.index.get_mut(&v, env) {
                    c.count += 1;
                } else {
                    b.index.insert(
                        v,
                        BagCell {
                            count: 1,
                            cell: new_cell(),
                        },
                        env,
                    );
                }
                true
            }
            Coll::Sequence(s) => {
                s.push(v);
                true
            }
        }
    }

    /// `addAll(c)`: `add` of each element of `c` in its iteration order.
    pub fn add_all(&mut self, c: &Coll, env: &dyn HashEnv) -> bool {
        let mut changed = false;
        if let (Coll::Sequence(dst), Coll::Sequence(src)) = (&mut *self, c) {
            dst.extend_from_slice(src);
            return !src.is_empty();
        }
        for v in c.iter() {
            changed |= self.add(v.clone(), env);
        }
        changed
    }

    /// Java `remove(Object)`: a bag drops all occurrences, a sequence the first.
    pub fn remove(&mut self, v: &Value, env: &dyn HashEnv) -> bool {
        match self {
            Coll::Set(s) => s.remove(v, env).is_some(),
            Coll::OrderedSet(s) => s.remove(v, env).is_some(),
            Coll::Bag(b) => match b.index.remove(v, env) {
                Some((_, c)) => {
                    b.size -= c.count as usize;
                    true
                }
                None => false,
            },
            Coll::Sequence(s) => match s.iter().position(|e| crate::eq::java_equals(e, v, env)) {
                Some(i) => {
                    s.remove(i);
                    true
                }
                None => false,
            },
        }
    }

    /// `clear()` (a `HashSet` keeps its table capacity).
    pub fn clear(&mut self) {
        match self {
            Coll::Set(s) => s.clear(),
            Coll::OrderedSet(s) => s.clear(),
            Coll::Bag(b) => {
                b.index.clear();
                b.size = 0;
            }
            Coll::Sequence(v) => v.clear(),
        }
    }

    /// Removal through the collection's iterator of every element for which `f` is false.
    /// `Err(())` for a bag (`BagImpl`'s iterator does not support removal).
    #[allow(clippy::result_unit_err)]
    pub fn retain_iter(
        &mut self,
        mut f: impl FnMut(&Value) -> bool,
        env: &dyn HashEnv,
    ) -> Result<bool, ()> {
        Ok(match self {
            Coll::Set(s) => s.retain(|v, _| f(v), env),
            Coll::OrderedSet(s) => s.retain(|v, _| f(v), env),
            Coll::Bag(_) => return Err(()),
            Coll::Sequence(v) => {
                let n = v.len();
                v.retain(|e| f(e));
                v.len() != n
            }
        })
    }

    /// Java `removeAll(c)`: `AbstractSet.removeAll` for sets, `ArrayList.removeAll` (every
    /// element `c` contains) for sequences; `Err(())` for a bag.
    #[allow(clippy::result_unit_err)]
    pub fn remove_all(&mut self, c: &Coll, env: &dyn HashEnv) -> Result<bool, ()> {
        match self {
            Coll::Set(_) | Coll::OrderedSet(_) => Ok(ops::remove_all(self, c, env)),
            Coll::Sequence(_) => self.retain_iter(|e| !c.contains(e, env), env),
            Coll::Bag(_) => Err(()),
        }
    }

    /// Java `contains` (hash lookup with Java `equals`; `Set{1}->includes(1.0)` is false).
    pub fn contains(&self, v: &Value, env: &dyn HashEnv) -> bool {
        match self {
            Coll::Set(s) => s.get(v, env).is_some(),
            Coll::OrderedSet(s) => s.get(v, env).is_some(),
            Coll::Bag(b) => b.index.get(v, env).is_some(),
            Coll::Sequence(s) => s.iter().any(|e| crate::eq::java_equals(e, v, env)),
        }
    }

    /// `BagImpl.count(o)`; for other kinds the number of Java-equal elements.
    pub fn java_count(&self, v: &Value, env: &dyn HashEnv) -> usize {
        match self {
            Coll::Bag(b) => b.index.get(v, env).map_or(0, |c| c.count as usize),
            Coll::Sequence(s) => s
                .iter()
                .filter(|e| crate::eq::java_equals(e, v, env))
                .count(),
            _ => self.contains(v, env) as usize,
        }
    }

    /// Java `hashCode()` of the collection.
    pub fn java_hash(&self, env: &dyn HashEnv) -> i32 {
        match self {
            Coll::Set(s) => s
                .iter()
                .fold(0i32, |h, (e, _)| h.wrapping_add(java_hash(e, env))),
            Coll::OrderedSet(s) => s
                .iter()
                .fold(0i32, |h, (e, _)| h.wrapping_add(java_hash(e, env))),
            Coll::Sequence(s) => list_hash(s.iter(), env),
            Coll::Bag(b) => {
                // 37 * (37 * 37 + HashMap<E, MutableInteger>.hashCode()) + size
                let map = b.index.iter().fold(0i32, |h, (e, c)| {
                    h.wrapping_add(
                        java_hash(e, env)
                            ^ env.identity_hash(crate::identity::IdentityKey::Cell(c.cell)),
                    )
                });
                37i32
                    .wrapping_mul(37i32.wrapping_mul(37).wrapping_add(map))
                    .wrapping_add(b.size as i32)
            }
        }
    }

    /// The storage of a `Set`/`Bag` keeps Java's bucket order.
    pub fn is_java_ordered(&self) -> bool {
        match self {
            Coll::Set(s) => s.is_hash(),
            Coll::Bag(b) => b.index.is_hash(),
            _ => true,
        }
    }
}

/// `AbstractList.hashCode` over the elements.
pub fn list_hash<'a>(it: impl Iterator<Item = &'a Value>, env: &dyn HashEnv) -> i32 {
    it.fold(1i32, |h, e| {
        h.wrapping_mul(31).wrapping_add(java_hash(e, env))
    })
}

/// Iterator over a collection's elements.
pub enum CollIter<'a> {
    Set(IndexIter<'a, ()>),
    OSet(linked::Iter<'a, ()>),
    Bag {
        it: IndexIter<'a, BagCell>,
        cur: Option<&'a Value>,
        left: u32,
    },
    Seq(std::slice::Iter<'a, Value>),
}

impl<'a> Iterator for CollIter<'a> {
    type Item = &'a Value;
    #[inline]
    fn next(&mut self) -> Option<&'a Value> {
        match self {
            CollIter::Set(i) => i.next().map(|(k, _)| k),
            CollIter::OSet(i) => i.next().map(|(k, _)| k),
            CollIter::Seq(i) => i.next(),
            CollIter::Bag { it, cur, left } => {
                if *left > 0 {
                    *left -= 1;
                    return *cur;
                }
                let (k, c) = it.next()?;
                *cur = Some(k);
                *left = c.count - 1;
                Some(k)
            }
        }
    }
}

impl std::fmt::Debug for Coll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind().ocl_name())?;
        f.debug_list().entries(self.iter()).finish()
    }
}
