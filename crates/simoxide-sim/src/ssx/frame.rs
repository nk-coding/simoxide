//! SimuCom stack frames with Java's identity semantics, for the Slingshot-exact engine.
//!
//! Slingshot passes `SimulatedStackframe` objects around by reference and writes into them
//! after other objects have captured them (result frames, the user's base frame, the frame of a
//! collection iterator), so the exact engine keeps frames as shared mutable objects
//! ([`SFrame`]), unlike the copy-on-write frames of the process interpreter
//! ([`crate::frames`]). Evaluation proxies (`INNER`) still capture an immutable snapshot
//! (`copyFrame`), which is a [`crate::frames::Frame`] chain, so proxy evaluation is shared code.

use crate::frames::{Binding, Frame, FrameRef, eval_proxy};
use crate::ir::{CompiledModel, KeyId};
use simoxide_random::UniformSource;
use simoxide_stoex::{Env, EvalError, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// `SimulatedStackframe`: entries in first-insertion order (a `put` of an existing key replaces
/// in place) and an optional parent.
#[derive(Default)]
pub(crate) struct JFrame {
    pub entries: Vec<(KeyId, Binding)>,
    pub parent: Option<SFrame>,
}

/// A frame object (Java reference semantics).
pub(crate) type SFrame = Rc<RefCell<JFrame>>;

pub(crate) fn new_frame(parent: Option<SFrame>) -> SFrame {
    Rc::new(RefCell::new(JFrame {
        entries: Vec::new(),
        parent,
    }))
}

impl JFrame {
    pub fn put(&mut self, key: KeyId, b: Binding) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.0 == key) {
            e.1 = b;
        } else {
            self.entries.push((key, b));
        }
    }
}

/// `getValue(id)` along the parent chain.
pub(crate) fn lookup(f: &SFrame, key: KeyId) -> Option<Binding> {
    let mut cur = f.clone();
    loop {
        let next = {
            let fr = cur.borrow();
            if let Some(e) = fr.entries.iter().find(|e| e.0 == key) {
                return Some(e.1.clone());
            }
            fr.parent.clone()
        };
        cur = next?;
    }
}

/// `copyFrame()`: a deep copy of the whole chain (values shared, maps new), as the immutable
/// frames proxies evaluate against.
pub(crate) fn snapshot(f: &SFrame) -> FrameRef {
    let mut chain: Vec<SFrame> = vec![f.clone()];
    loop {
        let p = chain[chain.len() - 1].borrow().parent.clone();
        match p {
            Some(p) => chain.push(p),
            None => break,
        }
    }
    let mut parent: Option<FrameRef> = None;
    for fr in chain.into_iter().rev() {
        let entries = fr.borrow().entries.clone();
        parent = Some(FrameRef::new(Frame { entries, parent }));
    }
    parent.expect("non-empty chain")
}

/// `getContents()`: own entries, then the parent's entries whose key was not seen yet,
/// recursively. SimuCom keeps a frame in a `java.util.HashMap`; its iteration order is defined
/// only by OpenJDK's sources, which the EPL build does not use, so the entries come in
/// first-insertion order (a documented deviation for frames with several `INNER` or `BYTESIZE`
/// entries, `docs/correctness/slingshot.md`).
pub(crate) fn contents(f: &SFrame) -> Vec<(KeyId, Binding)> {
    let mut out: Vec<(KeyId, Binding)> = Vec::new();
    let mut cur = Some(f.clone());
    while let Some(fr) = cur {
        let b = fr.borrow();
        for (k, v) in &b.entries {
            if !out.iter().any(|(x, _)| x == k) {
                out.push((*k, v.clone()));
            }
        }
        cur = b.parent.clone();
    }
    out
}

/// A frame (or none) as a StoEx environment.
pub(crate) struct SEnv<'a> {
    pub frame: Option<&'a SFrame>,
    pub cm: &'a CompiledModel,
}

impl Env for SEnv<'_> {
    fn lookup<R: UniformSource + ?Sized>(
        &self,
        slot: u32,
        rng: &mut R,
    ) -> Result<Option<Value>, EvalError> {
        let Some(f) = self.frame else {
            return Ok(None);
        };
        match lookup(f, slot) {
            None => Ok(None),
            Some(Binding::Val(v)) => Ok(Some(v)),
            Some(Binding::Proxy(p)) => eval_proxy(self.cm, &p, rng).map(Some),
        }
    }
}
