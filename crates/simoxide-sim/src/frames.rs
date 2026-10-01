//! Stack frames (`SimulatedStackframe`, spec ACT-5).
//!
//! A frame maps key ids to values or lazily evaluated proxies and has an optional parent. Frames
//! are shared through `Rc` and treated as immutable snapshots once shared: writing into a shared
//! frame (`Rc::make_mut`) copies it. This reproduces the reference, where `copyFrame()` makes deep
//! copies for proxies and fork children and only the top frame of a stack is ever written to.

use crate::ir::{CompiledModel, KeyId, Keys, ProgId};
use crate::javahash::{iteration_order, spread};
use simoxide_random::UniformSource;
use simoxide_stoex::{Env, EvalError, Value};
use std::rc::Rc;

/// `EvaluationProxy`: a specification evaluated against a frozen frame on every access.
#[derive(Debug)]
pub(crate) struct Proxy {
    pub prog: ProgId,
    pub frame: Option<Rc<Frame>>,
}

#[derive(Debug, Clone)]
pub(crate) enum Binding {
    Val(Value),
    Proxy(Rc<Proxy>),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Frame {
    /// Entries in first-insertion order (a `put` of an existing key replaces in place).
    pub entries: Vec<(KeyId, Binding)>,
    pub parent: Option<Rc<Frame>>,
}

impl Frame {
    pub fn with_parent(parent: Option<Rc<Frame>>) -> Frame {
        Frame {
            entries: Vec::new(),
            parent,
        }
    }

    pub fn put(&mut self, key: KeyId, b: Binding) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.0 == key) {
            e.1 = b;
        } else {
            self.entries.push((key, b));
        }
    }

    pub fn get(&self, key: KeyId) -> Option<&Binding> {
        let mut f = self;
        loop {
            if let Some(e) = f.entries.iter().find(|e| e.0 == key) {
                return Some(&e.1);
            }
            f = f.parent.as_deref()?;
        }
    }

    /// `getContents()`: own entries in `HashMap` order, then the parent's entries whose key was
    /// not seen yet, recursively.
    pub fn contents(&self, keys: &Keys) -> Vec<(KeyId, Binding)> {
        let mut seen: Vec<KeyId> = Vec::new();
        let mut out = Vec::new();
        let mut f = Some(self);
        while let Some(fr) = f {
            let hashes: Vec<i32> = fr
                .entries
                .iter()
                .map(|(k, _)| keys.hash[*k as usize])
                .collect();
            for i in iteration_order(&hashes) {
                let (k, b) = &fr.entries[i];
                if !seen.contains(k) {
                    seen.push(*k);
                    out.push((*k, b.clone()));
                }
            }
            f = fr.parent.as_deref();
        }
        out
    }

    /// Calls `f` for every entry of [`Frame::contents`], in the same order, without allocating
    /// for small frames.
    pub fn visit_contents<E>(
        &self,
        keys: &Keys,
        mut f: impl FnMut(KeyId, &Binding) -> Result<(), E>,
    ) -> Result<(), E> {
        const SEEN: usize = 32;
        let mut seen = [0 as KeyId; SEEN];
        let mut n_seen = 0usize;
        let mut seen_more: Vec<KeyId> = Vec::new();
        let mut fr = Some(self);
        while let Some(frame) = fr {
            let n = frame.entries.len();
            let mut visit = |i: usize| -> Result<(), E> {
                let (k, b) = &frame.entries[i];
                if seen[..n_seen].contains(k) || seen_more.contains(k) {
                    return Ok(());
                }
                if n_seen < SEEN {
                    seen[n_seen] = *k;
                    n_seen += 1;
                } else {
                    seen_more.push(*k);
                }
                f(*k, b)
            };
            if n <= 12 {
                // capacity 16: stable insertion sort of the entry indices by bucket
                let mut idx = [0u8; 12];
                let mut bucket = [0u32; 12];
                for i in 0..n {
                    let bk = spread(keys.hash[frame.entries[i].0 as usize]) & 15;
                    let mut j = i;
                    while j > 0 && bucket[j - 1] > bk {
                        bucket[j] = bucket[j - 1];
                        idx[j] = idx[j - 1];
                        j -= 1;
                    }
                    bucket[j] = bk;
                    idx[j] = i as u8;
                }
                for &i in &idx[..n] {
                    visit(i as usize)?;
                }
            } else {
                let hashes: Vec<i32> = frame
                    .entries
                    .iter()
                    .map(|(k, _)| keys.hash[*k as usize])
                    .collect();
                for i in iteration_order(&hashes) {
                    visit(i)?;
                }
            }
            fr = frame.parent.as_deref();
        }
        Ok(())
    }

    /// Any key of the chain satisfies `pred`.
    pub fn any_key(&self, mut pred: impl FnMut(KeyId) -> bool) -> bool {
        let mut f = Some(self);
        while let Some(fr) = f {
            if fr.entries.iter().any(|(k, _)| pred(*k)) {
                return true;
            }
            f = fr.parent.as_deref();
        }
        false
    }
}

/// Recycled frames: a frame whose last reference is popped from a process's frame stack keeps
/// its allocation (the `Rc` box and the entry vector) for the next frame. Only unshared frames
/// are recycled (`Rc::get_mut`), so no other reference can observe the reuse; everything else
/// is dropped as before. Values and lookups do not change.
#[derive(Default)]
pub(crate) struct FramePool {
    free: Vec<Rc<Frame>>,
}

/// The frame of an unshared `Rc` (a new frame of [`FramePool::frame`], or after
/// [`FramePool::unshare`]).
#[inline]
pub(crate) fn frame_mut(f: &mut Rc<Frame>) -> &mut Frame {
    Rc::get_mut(f).expect("unshared frame")
}

/// Recycled frames kept at most (a few call levels of a few processes).
const POOL_CAP: usize = 64;

impl FramePool {
    /// A new frame with no entries and parent `parent`.
    #[inline]
    pub fn frame(&mut self, parent: Option<Rc<Frame>>) -> Rc<Frame> {
        match self.free.pop() {
            Some(mut f) => {
                // pooled frames are unshared, empty and parentless
                if let Some(m) = Rc::get_mut(&mut f) {
                    m.parent = parent;
                }
                f
            }
            None => Rc::new(Frame::with_parent(parent)),
        }
    }

    /// `Rc::make_mut`: makes `f` unshared (a pooled copy if it is shared), for [`frame_mut`].
    #[inline]
    pub fn unshare(&mut self, f: &mut Rc<Frame>) {
        if Rc::get_mut(f).is_none() {
            let mut copy = self.frame(f.parent.clone());
            frame_mut(&mut copy)
                .entries
                .extend(f.entries.iter().cloned());
            *f = copy;
        }
    }

    /// [`FramePool::recycle`] for every frame of `v` (emptied, capacity kept).
    #[inline(never)]
    pub fn recycle_all(&mut self, v: &mut Vec<Rc<Frame>>) {
        for f in v.drain(..) {
            self.recycle(Some(f));
        }
    }

    /// Drops a frame reference, keeping the frame (and its unshared parents) for reuse when it
    /// was the last reference.
    #[inline(never)]
    pub fn recycle(&mut self, f: Option<Rc<Frame>>) {
        let mut next = f;
        while let Some(mut f) = next.take() {
            let Some(m) = Rc::get_mut(&mut f) else {
                return;
            };
            m.entries.clear();
            next = m.parent.take();
            if self.free.len() < POOL_CAP {
                self.free.push(f);
            }
        }
    }
}

/// A frame (or no frame) as a StoEx evaluation environment.
#[derive(Clone, Copy)]
pub(crate) struct FrameEnv<'a> {
    pub frame: Option<&'a Frame>,
    pub cm: &'a CompiledModel,
    /// Number of enclosing proxy evaluations (see [`MAX_PROXY_DEPTH`]).
    pub depth: u32,
}

/// Upper bound of nested `EvaluationProxy` evaluations (an `INNER` characterisation passed on
/// through nested calls evaluates the whole chain recursively). Deeper chains fail with an
/// evaluation error instead of overflowing the native stack; the reference already hangs at a
/// call depth of 300 (`docs/correctness/reference-bugs.md` REF-14, `docs/correctness/deviations.md`).
pub(crate) const MAX_PROXY_DEPTH: u32 = 2000;

impl Env for FrameEnv<'_> {
    #[inline]
    fn lookup<R: UniformSource + ?Sized>(
        &self,
        slot: u32,
        rng: &mut R,
    ) -> Result<Option<Value>, EvalError> {
        let Some(f) = self.frame else {
            return Ok(None);
        };
        match f.get(slot) {
            None => Ok(None),
            Some(Binding::Val(v)) => Ok(Some(v.clone())),
            Some(Binding::Proxy(p)) => eval_proxy_at(self.cm, p, rng, self.depth + 1).map(Some),
        }
    }
}

#[inline]
pub(crate) fn eval_proxy<R: UniformSource + ?Sized>(
    cm: &CompiledModel,
    p: &Proxy,
    rng: &mut R,
) -> Result<Value, EvalError> {
    eval_proxy_at(cm, p, rng, 0)
}

#[inline(never)]
fn eval_proxy_at<R: UniformSource + ?Sized>(
    cm: &CompiledModel,
    p: &Proxy,
    rng: &mut R,
    depth: u32,
) -> Result<Value, EvalError> {
    if depth > MAX_PROXY_DEPTH {
        return Err(EvalError::new(
            simoxide_stoex::EvalErrorKind::Runtime,
            format!(
                "limit exceeded: more than {MAX_PROXY_DEPTH} nested evaluation proxies (INNER characterisations passed through nested calls)"
            ),
        ));
    }
    let prog = match &cm.progs[p.prog as usize].prog {
        Ok(pr) => pr,
        Err(e) => {
            return Err(EvalError::new(
                simoxide_stoex::EvalErrorKind::Runtime,
                e.clone(),
            ));
        }
    };
    let env = FrameEnv {
        frame: p.frame.as_deref(),
        cm,
        depth,
    };
    prog.eval(&env, rng)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_replaces_in_place_and_lookup_walks_parents() {
        let mut parent = Frame::default();
        parent.put(1, Binding::Val(Value::Int(1)));
        parent.put(2, Binding::Val(Value::Int(2)));
        let parent = Rc::new(parent);
        let mut child = Frame::with_parent(Some(parent.clone()));
        child.put(2, Binding::Val(Value::Int(20)));
        child.put(2, Binding::Val(Value::Int(21)));
        assert_eq!(child.entries.len(), 1);
        match child.get(2) {
            Some(Binding::Val(Value::Int(21))) => {}
            other => panic!("{other:?}"),
        }
        match child.get(1) {
            Some(Binding::Val(Value::Int(1))) => {}
            other => panic!("{other:?}"),
        }
        assert!(child.get(3).is_none());
    }

    #[test]
    fn shared_frames_are_copied_on_write() {
        let mut f = Rc::new(Frame::default());
        let snapshot = f.clone();
        Rc::make_mut(&mut f).put(0, Binding::Val(Value::Int(5)));
        assert!(snapshot.entries.is_empty());
        assert_eq!(f.entries.len(), 1);
    }

    #[test]
    fn visit_contents_matches_contents() {
        let mut keys = Keys::default();
        let names: Vec<String> = (0..40)
            .map(|i| {
                format!(
                    "p{}.{}",
                    i * 7919 % 97,
                    if i % 3 == 0 { "BYTESIZE" } else { "VALUE" }
                )
            })
            .collect();
        let ids: Vec<KeyId> = names.iter().map(|k| keys.intern(k)).collect();
        // chains of frames of several sizes (crossing the 12-entry fast path) with overlaps
        for sizes in [[0usize, 3, 5], [12, 1, 0], [13, 20, 4], [2, 40, 12]] {
            let mut parent: Option<Rc<Frame>> = None;
            for (lvl, &n) in sizes.iter().enumerate() {
                let mut f = Frame::with_parent(parent.clone());
                for j in 0..n {
                    let k = ids[(j * (lvl + 3) + lvl) % ids.len()];
                    f.put(k, Binding::Val(Value::Int(j as i32)));
                }
                parent = Some(Rc::new(f));
            }
            let top = parent.expect("frame");
            let want: Vec<KeyId> = top.contents(&keys).into_iter().map(|(k, _)| k).collect();
            let mut got = Vec::new();
            top.visit_contents::<()>(&keys, |k, _| {
                got.push(k);
                Ok(())
            })
            .unwrap();
            assert_eq!(got, want, "sizes {sizes:?}");
        }
    }

    #[test]
    fn key_order_follows_java_hashmap() {
        let mut keys = Keys::default();
        // "Aa.VALUE" and "BB.VALUE" have equal String hash codes
        let ids: Vec<KeyId> = ["x.VALUE", "BB.VALUE", "Aa.VALUE", "y.BYTESIZE"]
            .iter()
            .map(|k| keys.intern(k))
            .collect();
        let hashes: Vec<i32> = ids.iter().map(|&k| keys.hash[k as usize]).collect();
        let order = iteration_order(&hashes);
        // colliding keys keep their insertion order
        let pos = |i: usize| order.iter().position(|&x| x == i).unwrap();
        assert!(pos(1) < pos(2));
        assert_eq!(
            crate::javahash::spread(keys.hash[ids[1] as usize]),
            crate::javahash::spread(keys.hash[ids[2] as usize])
        );
    }
}
