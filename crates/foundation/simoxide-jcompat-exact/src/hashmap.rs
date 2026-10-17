//! `java.util.HashMap` with Java's observable iteration order.
//!
//! An independent reimplementation of the behaviour of OpenJDK 21's `HashMap` (written from its
//! documented algorithm and checked against it by the differential tests in `tests/`), not a
//! copy of its source:
//!
//! - power-of-two table, bucket `spread(hashCode) & (capacity - 1)`, lazily allocated;
//! - the threshold rules of the constructors (`HashMap()`, `HashMap(int)`, `newHashMap(n)`,
//!   `HashMap(Map)` / `putAll`) and of `resize` (capacity doubles when `++size > threshold`;
//!   each bucket is split into a low and a high list keeping relative order);
//! - new keys are appended at the tail of their bucket's list;
//! - treeified bins: a bucket with more than 8 nodes becomes a red-black tree once the table has
//!   at least 64 buckets (otherwise the table grows). Iteration then follows the tree nodes'
//!   `next` links, which the tree code rearranges: the root moves to the front of the bucket,
//!   a new node is linked right after its tree parent, removal unlinks, a split or a removal can
//!   turn a small tree back into a list. All of this is reproduced, including the comparison
//!   rules (hash, `Comparable` class, class name, identity hash);
//! - removal through `remove(key)` ("movable") and through an iterator (not movable).
//!
//! Nodes live in an arena; links are `u32` indices.

use std::cmp::Ordering;

/// `HashMap.hash`: `h ^ (h >>> 16)`.
#[inline]
pub fn spread(h: i32) -> i32 {
    h ^ ((h as u32) >> 16) as i32
}

const NIL: u32 = u32::MAX;
const DEFAULT_CAPACITY: usize = 16;
const MAXIMUM_CAPACITY: usize = 1 << 30;
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;
const INT_MAX: usize = i32::MAX as usize;

/// A key with Java `hashCode`/`equals` semantics and the class information `HashMap`'s tree
/// bins use to order keys with equal hashes.
pub trait JavaKey<E: ?Sized> {
    fn java_hash(&self, env: &E) -> i32;
    fn java_equals(&self, other: &Self, env: &E) -> bool;
    /// `HashMap.comparableClassFor`: an id of the key's class if that class is `C implements
    /// Comparable<C>` (String and the boxed primitives), else `None` (also for `null`).
    fn comparable_class(&self) -> Option<u8>;
    /// `compareTo` between two keys of the same comparable class.
    fn compare_to(&self, other: &Self) -> Ordering;
    /// `getClass().getName()`; `None` for the `null` key.
    fn class_name<'a>(&'a self, env: &'a E) -> Option<&'a str>;
    /// `System.identityHashCode` (0 for `null`).
    fn identity_hash(&self, env: &E) -> i32;
}

#[derive(Clone)]
struct Node<K, V> {
    hash: i32,
    next: u32,
    // tree links, meaningful while `tree` is set
    tree: bool,
    red: bool,
    prev: u32,
    parent: u32,
    left: u32,
    right: u32,
    key: K,
    val: V,
}

/// `java.util.HashMap<K, V>` with load factor 0.75.
#[derive(Clone)]
pub struct JavaHashMap<K, V> {
    table: Vec<u32>,
    nodes: Vec<Option<Node<K, V>>>,
    free: Vec<u32>,
    size: usize,
    /// Java's `threshold` field (holds the initial capacity before the table is allocated).
    threshold: usize,
    /// An upper bound of the longest bucket list (lists only grow by insertion; resizes and
    /// removals shorten them).
    max_bin_hint: usize,
}

impl<K, V> Default for JavaHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

/// `HashMap.tableSizeFor`: the smallest power of two `>= cap` (1 for `cap <= 1`).
pub fn table_size_for(cap: usize) -> usize {
    if cap <= 1 {
        1
    } else {
        cap.next_power_of_two().min(MAXIMUM_CAPACITY)
    }
}

/// `HashMap.calculateHashMapCapacity(n)` (JDK 19+): `ceil(n / 0.75)`.
pub fn hash_map_capacity_for(num_mappings: usize) -> usize {
    (num_mappings as f64 / 0.75).ceil() as usize
}

impl<K, V> JavaHashMap<K, V> {
    /// `new HashMap()`.
    pub fn new() -> Self {
        JavaHashMap {
            table: Vec::new(),
            nodes: Vec::new(),
            free: Vec::new(),
            size: 0,
            threshold: 0,
            max_bin_hint: 0,
        }
    }

    /// `new HashMap(initialCapacity)`.
    pub fn with_initial_capacity(initial_capacity: usize) -> Self {
        let mut m = Self::new();
        m.threshold = table_size_for(initial_capacity);
        m
    }

    /// `HashMap.newHashMap(numMappings)` (JDK 19+), used by `new HashSet(Collection)`.
    pub fn new_hash_map(num_mappings: usize) -> Self {
        Self::with_initial_capacity(hash_map_capacity_for(num_mappings))
    }

    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    /// Current table length (0 before the first insertion).
    pub fn capacity(&self) -> usize {
        self.table.len()
    }
    /// Length of the longest bucket list.
    pub fn max_bin_len(&self) -> usize {
        let mut best = 0;
        for &h in &self.table {
            let mut n = 0;
            let mut p = h;
            while p != NIL {
                n += 1;
                p = self.n(p).next;
            }
            best = best.max(n);
        }
        best
    }
    /// Whether every bucket list is shorter than `n` (O(1) unless the insertion history
    /// leaves that open).
    pub fn bins_shorter_than(&self, n: usize) -> bool {
        self.max_bin_hint < n || self.max_bin_len() < n
    }

    /// True if some bucket is a red-black tree.
    pub fn has_tree_bins(&self) -> bool {
        // a bin becomes a tree only when its list grows beyond the threshold
        if self.max_bin_hint <= TREEIFY_THRESHOLD {
            return false;
        }
        self.table.iter().any(|&h| h != NIL && self.n(h).tree)
    }

    #[inline]
    fn n(&self, i: u32) -> &Node<K, V> {
        self.nodes[i as usize].as_ref().expect("live node")
    }
    #[inline]
    fn m(&mut self, i: u32) -> &mut Node<K, V> {
        self.nodes[i as usize].as_mut().expect("live node")
    }

    fn alloc(&mut self, hash: i32, key: K, val: V, next: u32) -> u32 {
        let node = Node {
            hash,
            next,
            tree: false,
            red: false,
            prev: NIL,
            parent: NIL,
            left: NIL,
            right: NIL,
            key,
            val,
        };
        if let Some(i) = self.free.pop() {
            self.nodes[i as usize] = Some(node);
            i
        } else {
            self.nodes.push(Some(node));
            (self.nodes.len() - 1) as u32
        }
    }

    fn release(&mut self, i: u32) -> (K, V) {
        let n = self.nodes[i as usize].take().expect("live node");
        self.free.push(i);
        (n.key, n.val)
    }

    /// `HashMap.clear()`: keeps the table.
    pub fn clear(&mut self) {
        for b in self.table.iter_mut() {
            *b = NIL;
        }
        self.nodes.clear();
        self.free.clear();
        self.size = 0;
        self.max_bin_hint = 0;
    }

    /// Entries in Java iteration order.
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            map: self,
            bucket: 0,
            node: NIL,
            left: self.size,
        }
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(k, _)| k)
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
    /// Arena ids of the entries in iteration order (stable while the entry lives).
    pub fn node_ids(&self) -> impl Iterator<Item = u32> + '_ {
        NodeIds {
            map: self,
            bucket: 0,
            node: NIL,
        }
    }
    pub fn node_key(&self, id: u32) -> &K {
        &self.n(id).key
    }
    pub fn node_value(&self, id: u32) -> &V {
        &self.n(id).val
    }
    pub fn node_value_mut(&mut self, id: u32) -> &mut V {
        &mut self.m(id).val
    }
    /// The cached spread hash of an entry.
    pub fn node_hash(&self, id: u32) -> i32 {
        self.n(id).hash
    }

    fn first_node_from(&self, mut bucket: usize) -> (usize, u32) {
        while bucket < self.table.len() {
            let h = self.table[bucket];
            if h != NIL {
                return (bucket, h);
            }
            bucket += 1;
        }
        (bucket, NIL)
    }

    // ------------------------------------------------------------------ tree helpers

    fn untreeify(&mut self, head: u32) -> u32 {
        let mut p = head;
        while p != NIL {
            let n = self.m(p);
            n.tree = false;
            n.prev = NIL;
            n.parent = NIL;
            n.left = NIL;
            n.right = NIL;
            n.red = false;
            p = n.next;
        }
        head
    }

    fn root_of(&self, mut r: u32) -> u32 {
        loop {
            let p = self.n(r).parent;
            if p == NIL {
                return r;
            }
            r = p;
        }
    }

    fn move_root_to_front(&mut self, root: u32) {
        let n = self.table.len();
        if root == NIL || n == 0 {
            return;
        }
        let index = (n - 1) & (self.n(root).hash as u32 as usize);
        let first = self.table[index];
        if root != first {
            self.table[index] = root;
            let rp = self.n(root).prev;
            let rn = self.n(root).next;
            if rn != NIL {
                self.m(rn).prev = rp;
            }
            if rp != NIL {
                self.m(rp).next = rn;
            }
            if first != NIL {
                self.m(first).prev = root;
            }
            self.m(root).next = first;
            self.m(root).prev = NIL;
        }
    }

    fn rotate_left(&mut self, mut root: u32, p: u32) -> u32 {
        if p == NIL {
            return root;
        }
        let r = self.n(p).right;
        if r == NIL {
            return root;
        }
        let rl = self.n(r).left;
        self.m(p).right = rl;
        if rl != NIL {
            self.m(rl).parent = p;
        }
        let pp = self.n(p).parent;
        self.m(r).parent = pp;
        if pp == NIL {
            root = r;
            self.m(r).red = false;
        } else if self.n(pp).left == p {
            self.m(pp).left = r;
        } else {
            self.m(pp).right = r;
        }
        self.m(r).left = p;
        self.m(p).parent = r;
        root
    }

    fn rotate_right(&mut self, mut root: u32, p: u32) -> u32 {
        if p == NIL {
            return root;
        }
        let l = self.n(p).left;
        if l == NIL {
            return root;
        }
        let lr = self.n(l).right;
        self.m(p).left = lr;
        if lr != NIL {
            self.m(lr).parent = p;
        }
        let pp = self.n(p).parent;
        self.m(l).parent = pp;
        if pp == NIL {
            root = l;
            self.m(l).red = false;
        } else if self.n(pp).right == p {
            self.m(pp).right = l;
        } else {
            self.m(pp).left = l;
        }
        self.m(l).right = p;
        self.m(p).parent = l;
        root
    }

    fn red(&self, x: u32) -> bool {
        x != NIL && self.n(x).red
    }

    fn parent(&self, x: u32) -> u32 {
        if x == NIL { NIL } else { self.n(x).parent }
    }

    fn balance_insertion(&mut self, mut root: u32, mut x: u32) -> u32 {
        self.m(x).red = true;
        loop {
            let mut xp = self.n(x).parent;
            if xp == NIL {
                self.m(x).red = false;
                return x;
            }
            if !self.n(xp).red {
                return root;
            }
            let mut xpp = self.n(xp).parent;
            if xpp == NIL {
                return root;
            }
            let xppl = self.n(xpp).left;
            if xp == xppl {
                let xppr = self.n(xpp).right;
                if xppr != NIL && self.n(xppr).red {
                    self.m(xppr).red = false;
                    self.m(xp).red = false;
                    self.m(xpp).red = true;
                    x = xpp;
                } else {
                    if x == self.n(xp).right {
                        x = xp;
                        root = self.rotate_left(root, x);
                        xp = self.n(x).parent;
                        xpp = self.parent(xp);
                    }
                    if xp != NIL {
                        self.m(xp).red = false;
                        if xpp != NIL {
                            self.m(xpp).red = true;
                            root = self.rotate_right(root, xpp);
                        }
                    }
                }
            } else if xppl != NIL && self.n(xppl).red {
                self.m(xppl).red = false;
                self.m(xp).red = false;
                self.m(xpp).red = true;
                x = xpp;
            } else {
                if x == self.n(xp).left {
                    x = xp;
                    root = self.rotate_right(root, x);
                    xp = self.n(x).parent;
                    xpp = self.parent(xp);
                }
                if xp != NIL {
                    self.m(xp).red = false;
                    if xpp != NIL {
                        self.m(xpp).red = true;
                        root = self.rotate_left(root, xpp);
                    }
                }
            }
        }
    }

    fn balance_deletion(&mut self, mut root: u32, mut x: u32) -> u32 {
        loop {
            if x == NIL || x == root {
                return root;
            }
            let mut xp = self.n(x).parent;
            if xp == NIL {
                self.m(x).red = false;
                return x;
            }
            if self.n(x).red {
                self.m(x).red = false;
                return root;
            }
            let xpl = self.n(xp).left;
            if xpl == x {
                let mut xpr = self.n(xp).right;
                if xpr != NIL && self.n(xpr).red {
                    self.m(xpr).red = false;
                    self.m(xp).red = true;
                    root = self.rotate_left(root, xp);
                    xp = self.n(x).parent;
                    xpr = if xp == NIL { NIL } else { self.n(xp).right };
                }
                if xpr == NIL {
                    x = xp;
                } else {
                    let sl = self.n(xpr).left;
                    let mut sr = self.n(xpr).right;
                    if !self.red(sr) && !self.red(sl) {
                        self.m(xpr).red = true;
                        x = xp;
                    } else {
                        if !self.red(sr) {
                            if sl != NIL {
                                self.m(sl).red = false;
                            }
                            self.m(xpr).red = true;
                            root = self.rotate_right(root, xpr);
                            xp = self.n(x).parent;
                            xpr = if xp == NIL { NIL } else { self.n(xp).right };
                        }
                        if xpr != NIL {
                            let c = if xp == NIL { false } else { self.n(xp).red };
                            self.m(xpr).red = c;
                            sr = self.n(xpr).right;
                            if sr != NIL {
                                self.m(sr).red = false;
                            }
                        }
                        if xp != NIL {
                            self.m(xp).red = false;
                            root = self.rotate_left(root, xp);
                        }
                        x = root;
                    }
                }
            } else {
                let mut xpl = xpl;
                if xpl != NIL && self.n(xpl).red {
                    self.m(xpl).red = false;
                    self.m(xp).red = true;
                    root = self.rotate_right(root, xp);
                    xp = self.n(x).parent;
                    xpl = if xp == NIL { NIL } else { self.n(xp).left };
                }
                if xpl == NIL {
                    x = xp;
                } else {
                    let mut sl = self.n(xpl).left;
                    let sr = self.n(xpl).right;
                    if !self.red(sl) && !self.red(sr) {
                        self.m(xpl).red = true;
                        x = xp;
                    } else {
                        if !self.red(sl) {
                            if sr != NIL {
                                self.m(sr).red = false;
                            }
                            self.m(xpl).red = true;
                            root = self.rotate_left(root, xpl);
                            xp = self.n(x).parent;
                            xpl = if xp == NIL { NIL } else { self.n(xp).left };
                        }
                        if xpl != NIL {
                            let c = if xp == NIL { false } else { self.n(xp).red };
                            self.m(xpl).red = c;
                            sl = self.n(xpl).left;
                            if sl != NIL {
                                self.m(sl).red = false;
                            }
                        }
                        if xp != NIL {
                            self.m(xp).red = false;
                            root = self.rotate_right(root, xp);
                        }
                        x = root;
                    }
                }
            }
        }
    }
}

impl<K, V> JavaHashMap<K, V> {
    /// `HashMap.tieBreakOrder`.
    fn tie_break_order<E: ?Sized>(&self, a: &K, b: &K, env: &E) -> Ordering
    where
        K: JavaKey<E>,
    {
        let d = match (a.class_name(env), b.class_name(env)) {
            (Some(x), Some(y)) => x.cmp(y),
            _ => Ordering::Equal,
        };
        if d != Ordering::Equal {
            return d;
        }
        if a.identity_hash(env) <= b.identity_hash(env) {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

    /// `compareComparables(kc, k, x)`: 0 unless `x` has class `kc`.
    fn compare_comparables<E: ?Sized>(kc: u8, k: &K, x: &K) -> Ordering
    where
        K: JavaKey<E>,
    {
        if x.comparable_class() != Some(kc) {
            Ordering::Equal
        } else {
            k.compare_to(x)
        }
    }

    // ------------------------------------------------------------------ resize

    fn resize<E: ?Sized>(&mut self, env: &E)
    where
        K: JavaKey<E>,
    {
        let old_cap = self.table.len();
        let old_thr = self.threshold;
        let new_cap;
        let mut new_thr = 0usize;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = INT_MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr;
        } else {
            new_cap = DEFAULT_CAPACITY;
            new_thr = (0.75f32 * DEFAULT_CAPACITY as f32) as usize;
        }
        if new_thr == 0 {
            let ft = new_cap as f32 * 0.75f32;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 {
                ft as usize
            } else {
                INT_MAX
            };
        }
        self.threshold = new_thr;
        let old = std::mem::replace(&mut self.table, vec![NIL; new_cap]);
        if old_cap == 0 {
            return;
        }
        for (j, &e) in old.iter().enumerate() {
            if e == NIL {
                continue;
            }
            if self.n(e).next == NIL {
                let h = self.n(e).hash as u32 as usize;
                self.table[h & (new_cap - 1)] = e;
            } else if self.n(e).tree {
                self.split_tree(e, j, old_cap, env);
            } else {
                let (mut lo_h, mut lo_t, mut hi_h, mut hi_t) = (NIL, NIL, NIL, NIL);
                let mut p = e;
                while p != NIL {
                    let next = self.n(p).next;
                    if (self.n(p).hash as u32 as usize) & old_cap == 0 {
                        if lo_t == NIL {
                            lo_h = p
                        } else {
                            self.m(lo_t).next = p
                        }
                        lo_t = p;
                    } else {
                        if hi_t == NIL {
                            hi_h = p
                        } else {
                            self.m(hi_t).next = p
                        }
                        hi_t = p;
                    }
                    p = next;
                }
                if lo_t != NIL {
                    self.m(lo_t).next = NIL;
                    self.table[j] = lo_h;
                }
                if hi_t != NIL {
                    self.m(hi_t).next = NIL;
                    self.table[j + old_cap] = hi_h;
                }
            }
        }
    }

    fn treeify<E: ?Sized>(&mut self, head: u32, env: &E)
    where
        K: JavaKey<E>,
    {
        let mut root = NIL;
        let mut x = head;
        while x != NIL {
            let next = self.n(x).next;
            {
                let n = self.m(x);
                n.left = NIL;
                n.right = NIL;
            }
            if root == NIL {
                let n = self.m(x);
                n.parent = NIL;
                n.red = false;
                root = x;
            } else {
                let h = self.n(x).hash;
                let mut kc: Option<Option<u8>> = None;
                let mut p = root;
                loop {
                    let ph = self.n(p).hash;
                    let dir = if ph > h {
                        Ordering::Less
                    } else if ph < h {
                        Ordering::Greater
                    } else {
                        let kcv = *kc.get_or_insert_with(|| self.n(x).key.comparable_class());
                        let d = match kcv {
                            None => Ordering::Equal,
                            Some(c) => Self::compare_comparables(c, &self.n(x).key, &self.n(p).key),
                        };
                        if d == Ordering::Equal {
                            self.tie_break_order(&self.n(x).key, &self.n(p).key, env)
                        } else {
                            d
                        }
                    };
                    let xp = p;
                    p = if dir != Ordering::Greater {
                        self.n(p).left
                    } else {
                        self.n(p).right
                    };
                    if p == NIL {
                        self.m(x).parent = xp;
                        if dir != Ordering::Greater {
                            self.m(xp).left = x;
                        } else {
                            self.m(xp).right = x;
                        }
                        root = self.balance_insertion(root, x);
                        break;
                    }
                }
            }
            x = next;
        }
        self.move_root_to_front(root);
    }

    fn treeify_bin<E: ?Sized>(&mut self, hash: i32, env: &E)
    where
        K: JavaKey<E>,
    {
        let n = self.table.len();
        if n < MIN_TREEIFY_CAPACITY {
            self.resize(env);
            return;
        }
        let index = (n - 1) & (hash as u32 as usize);
        let hd = self.table[index];
        if hd == NIL {
            return;
        }
        let mut prev = NIL;
        let mut e = hd;
        while e != NIL {
            let node = self.m(e);
            node.tree = true;
            node.prev = prev;
            node.parent = NIL;
            node.left = NIL;
            node.right = NIL;
            node.red = false;
            prev = e;
            e = node.next;
        }
        self.treeify(hd, env);
    }

    fn split_tree<E: ?Sized>(&mut self, b: u32, index: usize, bit: usize, env: &E)
    where
        K: JavaKey<E>,
    {
        let (mut lo_h, mut lo_t, mut hi_h, mut hi_t) = (NIL, NIL, NIL, NIL);
        let (mut lc, mut hc) = (0usize, 0usize);
        let mut e = b;
        while e != NIL {
            let next = self.n(e).next;
            self.m(e).next = NIL;
            if (self.n(e).hash as u32 as usize) & bit == 0 {
                self.m(e).prev = lo_t;
                if lo_t == NIL {
                    lo_h = e
                } else {
                    self.m(lo_t).next = e
                }
                lo_t = e;
                lc += 1;
            } else {
                self.m(e).prev = hi_t;
                if hi_t == NIL {
                    hi_h = e
                } else {
                    self.m(hi_t).next = e
                }
                hi_t = e;
                hc += 1;
            }
            e = next;
        }
        if lo_h != NIL {
            if lc <= UNTREEIFY_THRESHOLD {
                let h = self.untreeify(lo_h);
                self.table[index] = h;
            } else {
                self.table[index] = lo_h;
                if hi_h != NIL {
                    self.treeify(lo_h, env);
                }
            }
        }
        if hi_h != NIL {
            if hc <= UNTREEIFY_THRESHOLD {
                let h = self.untreeify(hi_h);
                self.table[index + bit] = h;
            } else {
                self.table[index + bit] = hi_h;
                if lo_h != NIL {
                    self.treeify(hi_h, env);
                }
            }
        }
    }
}

/// Result of [`JavaHashMap::insert_node`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Inserted {
    /// The key was new; the entry's node id.
    New(u32),
    /// The key was present; the existing entry's node id (its key and value are unchanged).
    Existing(u32),
}

impl<K, V> JavaHashMap<K, V> {
    /// `HashMap.hash(key)`.
    #[inline]
    pub fn hash_of<E: ?Sized>(key: &K, env: &E) -> i32
    where
        K: JavaKey<E>,
    {
        spread(key.java_hash(env))
    }

    /// `put(key, val)`: returns the previous value; an existing entry keeps its key.
    pub fn put<E: ?Sized>(&mut self, key: K, val: V, env: &E) -> Option<V>
    where
        K: JavaKey<E>,
    {
        let hash = Self::hash_of(&key, env);
        match self.put_hashed(hash, key, val, env) {
            (Inserted::New(_), _) => None,
            (Inserted::Existing(_), old) => old,
        }
    }

    /// `putVal` with a precomputed spread hash; returns the node and, for an existing key, the
    /// replaced value.
    pub fn put_hashed<E: ?Sized>(
        &mut self,
        hash: i32,
        key: K,
        val: V,
        env: &E,
    ) -> (Inserted, Option<V>)
    where
        K: JavaKey<E>,
    {
        match self.insert_node(hash, key, val, env) {
            Ok(id) => (Inserted::New(id), None),
            Err((id, val)) => {
                let old = std::mem::replace(&mut self.m(id).val, val);
                (Inserted::Existing(id), Some(old))
            }
        }
    }

    /// `putIfAbsent`-like insertion that never touches an existing entry: `Ok(new node)` or
    /// `Err((existing node, val))`.
    pub fn insert_node<E: ?Sized>(
        &mut self,
        hash: i32,
        key: K,
        val: V,
        env: &E,
    ) -> Result<u32, (u32, V)>
    where
        K: JavaKey<E>,
    {
        if self.table.is_empty() {
            self.resize(env);
        }
        let n = self.table.len();
        let i = (n - 1) & (hash as u32 as usize);
        let p = self.table[i];
        let new_id;
        if p == NIL {
            new_id = self.alloc(hash, key, val, NIL);
            self.table[i] = new_id;
        } else if self.n(p).hash == hash && self.n(p).key.java_equals(&key, env) {
            return Err((p, val));
        } else if self.n(p).tree {
            {
                let id = self.put_tree_val(p, hash, key, val, env)?;
                new_id = id
            }
        } else {
            let mut p = p;
            let mut bin_count = 0usize;
            loop {
                let e = self.n(p).next;
                if e == NIL {
                    new_id = self.alloc(hash, key, val, NIL);
                    self.m(p).next = new_id;
                    self.max_bin_hint = self.max_bin_hint.max(bin_count + 2);
                    if bin_count >= TREEIFY_THRESHOLD - 1 {
                        self.treeify_bin(hash, env);
                    }
                    break;
                }
                if self.n(e).hash == hash && self.n(e).key.java_equals(&key, env) {
                    return Err((e, val));
                }
                p = e;
                bin_count += 1;
            }
        }
        self.size += 1;
        if self.size > self.threshold {
            self.resize(env);
        }
        Ok(new_id)
    }

    fn put_tree_val<E: ?Sized>(
        &mut self,
        bin_head: u32,
        h: i32,
        k: K,
        v: V,
        env: &E,
    ) -> Result<u32, (u32, V)>
    where
        K: JavaKey<E>,
    {
        let mut kc: Option<Option<u8>> = None;
        let mut searched = false;
        let root = if self.n(bin_head).parent != NIL {
            self.root_of(bin_head)
        } else {
            bin_head
        };
        let mut p = root;
        loop {
            let ph = self.n(p).hash;
            let dir;
            if ph > h {
                dir = Ordering::Less;
            } else if ph < h {
                dir = Ordering::Greater;
            } else if self.n(p).key.java_equals(&k, env) {
                return Err((p, v));
            } else {
                let kcv = *kc.get_or_insert_with(|| k.comparable_class());
                let d = match kcv {
                    None => Ordering::Equal,
                    Some(c) => Self::compare_comparables(c, &k, &self.n(p).key),
                };
                if d == Ordering::Equal {
                    if !searched {
                        searched = true;
                        let pl = self.n(p).left;
                        let pr = self.n(p).right;
                        if pl != NIL
                            && let Some(q) = self.find(pl, h, &k, kcv, env)
                        {
                            return Err((q, v));
                        }
                        if pr != NIL
                            && let Some(q) = self.find(pr, h, &k, kcv, env)
                        {
                            return Err((q, v));
                        }
                    }
                    dir = self.tie_break_order(&k, &self.n(p).key, env);
                } else {
                    dir = d;
                }
            }
            let xp = p;
            p = if dir != Ordering::Greater {
                self.n(xp).left
            } else {
                self.n(xp).right
            };
            if p == NIL {
                let xpn = self.n(xp).next;
                let x = self.alloc(h, k, v, xpn);
                {
                    let n = self.m(x);
                    n.tree = true;
                    n.parent = xp;
                    n.prev = xp;
                }
                if dir != Ordering::Greater {
                    self.m(xp).left = x;
                } else {
                    self.m(xp).right = x;
                }
                self.m(xp).next = x;
                if xpn != NIL {
                    self.m(xpn).prev = x;
                }
                let r = self.balance_insertion(root, x);
                self.move_root_to_front(r);
                return Ok(x);
            }
        }
    }

    /// `TreeNode.find(h, k, kc)` starting at `p`.
    fn find<E: ?Sized>(&self, start: u32, h: i32, k: &K, mut kc: Option<u8>, env: &E) -> Option<u32>
    where
        K: JavaKey<E>,
    {
        let mut p = start;
        let mut kc_known = kc.is_some();
        while p != NIL {
            let node = self.n(p);
            let (pl, pr) = (node.left, node.right);
            let ph = node.hash;
            if ph > h {
                p = pl;
            } else if ph < h {
                p = pr;
            } else if node.key.java_equals(k, env) {
                return Some(p);
            } else if pl == NIL {
                p = pr;
            } else if pr == NIL {
                p = pl;
            } else {
                if !kc_known {
                    kc = k.comparable_class();
                    kc_known = kc.is_some();
                }
                let dir = match kc {
                    Some(c) => Self::compare_comparables(c, k, &node.key),
                    None => Ordering::Equal,
                };
                if dir != Ordering::Equal {
                    p = if dir == Ordering::Less { pl } else { pr };
                } else if let Some(q) = self.find(pr, h, k, kc, env) {
                    return Some(q);
                } else {
                    p = pl;
                }
            }
        }
        None
    }

    /// Node id of `key`, if present.
    pub fn find_node<E: ?Sized>(&self, key: &K, env: &E) -> Option<u32>
    where
        K: JavaKey<E>,
    {
        self.find_node_hashed(Self::hash_of(key, env), key, env)
    }

    /// `getNode(hash, key)`.
    pub fn find_node_hashed<E: ?Sized>(&self, hash: i32, key: &K, env: &E) -> Option<u32>
    where
        K: JavaKey<E>,
    {
        let n = self.table.len();
        if n == 0 {
            return None;
        }
        let first = self.table[(n - 1) & (hash as u32 as usize)];
        if first == NIL {
            return None;
        }
        if self.n(first).hash == hash && self.n(first).key.java_equals(key, env) {
            return Some(first);
        }
        let e = self.n(first).next;
        if e == NIL {
            return None;
        }
        if self.n(first).tree {
            let root = self.root_of(first);
            return self.find(root, hash, key, None, env);
        }
        let mut e = e;
        while e != NIL {
            if self.n(e).hash == hash && self.n(e).key.java_equals(key, env) {
                return Some(e);
            }
            e = self.n(e).next;
        }
        None
    }

    pub fn get<E: ?Sized>(&self, key: &K, env: &E) -> Option<&V>
    where
        K: JavaKey<E>,
    {
        self.find_node(key, env).map(|i| &self.n(i).val)
    }
    pub fn get_mut<E: ?Sized>(&mut self, key: &K, env: &E) -> Option<&mut V>
    where
        K: JavaKey<E>,
    {
        match self.find_node(key, env) {
            Some(i) => Some(&mut self.m(i).val),
            None => None,
        }
    }
    pub fn contains_key<E: ?Sized>(&self, key: &K, env: &E) -> bool
    where
        K: JavaKey<E>,
    {
        self.find_node(key, env).is_some()
    }

    /// `remove(key)` (`removeNode(..., movable = true)`).
    pub fn remove<E: ?Sized>(&mut self, key: &K, env: &E) -> Option<(K, V)>
    where
        K: JavaKey<E>,
    {
        let id = self.find_node(key, env)?;
        Some(self.remove_node(id, true, env))
    }

    /// Removes an entry by node id. `movable = false` is removal through an iterator, which
    /// neither moves a tree root to the front nor turns a small tree back into a list.
    pub fn remove_node<E: ?Sized>(&mut self, node: u32, movable: bool, env: &E) -> (K, V)
    where
        K: JavaKey<E>,
    {
        let n = self.table.len();
        let hash = self.n(node).hash;
        let index = (n - 1) & (hash as u32 as usize);
        let p = self.table[index];
        if self.n(node).tree {
            self.remove_tree_node(node, index, movable);
        } else if node == p {
            self.table[index] = self.n(node).next;
        } else {
            let mut q = p;
            while self.n(q).next != node {
                q = self.n(q).next;
            }
            self.m(q).next = self.n(node).next;
        }
        let _ = env;
        self.size -= 1;
        self.release(node)
    }

    fn remove_tree_node<E: ?Sized>(&mut self, this: u32, index: usize, movable: bool)
    where
        K: JavaKey<E>,
    {
        let mut first = self.table[index];
        let mut root = first;
        let succ = self.n(this).next;
        let pred = self.n(this).prev;
        if pred == NIL {
            self.table[index] = succ;
            first = succ;
        } else {
            self.m(pred).next = succ;
        }
        if succ != NIL {
            self.m(succ).prev = pred;
        }
        if first == NIL {
            return;
        }
        if self.n(root).parent != NIL {
            root = self.root_of(root);
        }
        if root == NIL
            || (movable && {
                let rr = self.n(root).right;
                let rl = self.n(root).left;
                rr == NIL || rl == NIL || self.n(rl).left == NIL
            })
        {
            let h = self.untreeify(first);
            self.table[index] = h;
            return;
        }
        let p = this;
        let pl = self.n(p).left;
        let pr = self.n(p).right;

        let replacement = if pl != NIL && pr != NIL {
            let mut s = pr;
            loop {
                let sl = self.n(s).left;
                if sl == NIL {
                    break;
                }
                s = sl;
            }
            let c = self.n(s).red;
            let pred_red = self.n(p).red;
            self.m(s).red = pred_red;
            self.m(p).red = c;
            let sr = self.n(s).right;
            let pp = self.n(p).parent;
            if s == pr {
                self.m(p).parent = s;
                self.m(s).right = p;
            } else {
                let sp = self.n(s).parent;
                self.m(p).parent = sp;
                if sp != NIL {
                    if s == self.n(sp).left {
                        self.m(sp).left = p;
                    } else {
                        self.m(sp).right = p;
                    }
                }
                self.m(s).right = pr;
                if pr != NIL {
                    self.m(pr).parent = s;
                }
            }
            self.m(p).left = NIL;
            self.m(p).right = sr;
            if sr != NIL {
                self.m(sr).parent = p;
            }
            self.m(s).left = pl;
            if pl != NIL {
                self.m(pl).parent = s;
            }
            self.m(s).parent = pp;
            if pp == NIL {
                root = s;
            } else if p == self.n(pp).left {
                self.m(pp).left = s;
            } else {
                self.m(pp).right = s;
            }
            if sr != NIL { sr } else { p }
        } else if pl != NIL {
            pl
        } else if pr != NIL {
            pr
        } else {
            p
        };
        if replacement != p {
            let pp = self.n(p).parent;
            self.m(replacement).parent = pp;
            if pp == NIL {
                root = replacement;
                self.m(replacement).red = false;
            } else if p == self.n(pp).left {
                self.m(pp).left = replacement;
            } else {
                self.m(pp).right = replacement;
            }
            let n = self.m(p);
            n.left = NIL;
            n.right = NIL;
            n.parent = NIL;
        }
        let r = if self.n(p).red {
            root
        } else {
            self.balance_deletion(root, replacement)
        };
        if replacement == p {
            let pp = self.n(p).parent;
            self.m(p).parent = NIL;
            if pp != NIL {
                if p == self.n(pp).left {
                    self.m(pp).left = NIL;
                } else if p == self.n(pp).right {
                    self.m(pp).right = NIL;
                }
            }
        }
        if movable {
            self.move_root_to_front(r);
        }
    }

    /// Removes, in iteration order, every entry for which `f` is false (`Iterator.remove`).
    pub fn retain<E: ?Sized>(&mut self, mut f: impl FnMut(&K, &V) -> bool, env: &E) -> bool
    where
        K: JavaKey<E>,
    {
        let ids: Vec<u32> = self.node_ids().collect();
        let mut changed = false;
        for id in ids {
            let keep = {
                let n = self.n(id);
                f(&n.key, &n.val)
            };
            if !keep {
                self.remove_node(id, false, env);
                changed = true;
            }
        }
        changed
    }

    /// `putMapEntries(m)` as used by `new HashMap(Map)` and `putAll` (JDK 19+ sizing).
    pub fn put_all<E: ?Sized, I>(&mut self, entries: I, count: usize, env: &E)
    where
        K: JavaKey<E>,
        I: IntoIterator<Item = (K, V)>,
    {
        if count > 0 {
            if self.table.is_empty() {
                let t = ((count as f64) / 0.75).ceil();
                let t = if t < MAXIMUM_CAPACITY as f64 {
                    t as usize
                } else {
                    MAXIMUM_CAPACITY
                };
                if t > self.threshold {
                    self.threshold = table_size_for(t);
                }
            } else {
                while count > self.threshold && self.table.len() < MAXIMUM_CAPACITY {
                    self.resize(env);
                }
            }
        }
        for (k, v) in entries {
            self.put(k, v, env);
        }
    }

    /// Checks the red-black and list invariants of every tree bin (tests).
    pub fn check_invariants<E: ?Sized>(&self) -> Result<(), String>
    where
        K: JavaKey<E>,
    {
        let mut count = 0;
        for (b, &head) in self.table.iter().enumerate() {
            let mut p = head;
            let mut prev = NIL;
            while p != NIL {
                let node = self.n(p);
                if (node.hash as u32 as usize) & (self.table.len() - 1) != b {
                    return Err(format!("node in wrong bucket {b}"));
                }
                if node.tree != self.n(head).tree {
                    return Err("mixed tree and list nodes".into());
                }
                if node.tree && node.prev != prev {
                    return Err(format!("bad prev link in bucket {b}"));
                }
                count += 1;
                prev = p;
                p = node.next;
            }
        }
        if count != self.size {
            return Err(format!("size {} but {} nodes", self.size, count));
        }
        Ok(())
    }
}

/// Iterator over entries in Java order.
pub struct Iter<'a, K, V> {
    map: &'a JavaHashMap<K, V>,
    bucket: usize,
    node: u32,
    left: usize,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        if self.node == NIL {
            let (b, n) = self.map.first_node_from(self.bucket);
            if n == NIL {
                return None;
            }
            self.bucket = b + 1;
            self.node = n;
        }
        let node = self.map.n(self.node);
        self.node = node.next;
        self.left -= 1;
        Some((&node.key, &node.val))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

struct NodeIds<'a, K, V> {
    map: &'a JavaHashMap<K, V>,
    bucket: usize,
    node: u32,
}

impl<K, V> Iterator for NodeIds<'_, K, V> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.node == NIL {
            let (b, n) = self.map.first_node_from(self.bucket);
            if n == NIL {
                return None;
            }
            self.bucket = b + 1;
            self.node = n;
        }
        let id = self.node;
        self.node = self.map.n(id).next;
        Some(id)
    }
}
