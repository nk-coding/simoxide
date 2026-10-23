//! Interpreter listeners in the Slingshot-exact engine (SIM-14; see
//! `crate::listener` for the mapping of Slingshot's bus events to the callbacks).
//!
//! Slingshot has no processes: a *process* is a `User` (pid = its id) or the synchronous
//! forked behaviours of one `ForkAction` execution (pid = [`FORK_PID`] | the id of the first
//! child context; Slingshot runs them on the user's stack with a shared cursor, Q23).
//! Asynchronous forked behaviours are not interpreted (Q23): no process. The callbacks are
//! derived from the dispatched events before their handlers run; the bookkeeping here exists
//! only while a listener is registered and never touches the run.

use super::*;
use crate::listener::{AssemblyOp, Listener, ProcessKind};
use std::collections::HashSet;

/// Pids of fork processes (users count from 1).
const FORK_PID: u64 = 1 << 62;

/// The listener and what it needs to map events to callbacks.
#[derive(Default)]
pub(crate) struct Listening {
    pub l: Option<Box<dyn Listener>>,
    /// Users with a `process_created`.
    users: HashSet<u64>,
    /// Root SEFF interpretations in progress (holder identity) -> their operation and process.
    ops: HashMap<usize, (AssemblyOp, u64)>,
    /// Fork holders (identity) -> their process.
    forks: HashMap<usize, u64>,
}

fn hid(h: &HRef) -> usize {
    Rc::as_ptr(h) as *const () as usize
}

impl Engine<'_> {
    /// The process of SEFF context `c`: the fork it runs in, else the user.
    fn listen_pid(&self, c: &SeffCtx) -> u64 {
        let mut cur = Some(c);
        while let Some(x) = cur {
            if let Some(&p) = self.lis.forks.get(&hid(&x.holder)) {
                return p;
            }
            cur = x.parent.as_deref().or(x.caller.as_deref());
        }
        c.rpc.user.id
    }

    /// The provided role an entry-level call with system role `role` delegates to.
    fn listen_inner_role(&self, role: Option<RoleId>) -> Option<RoleId> {
        let m = self.m;
        let rid = &*m.roles[role?].id;
        m.structures[self.sys]
            .connectors
            .iter()
            .find_map(|&c| match &m.connectors[c].kind {
                ConnectorKind::ProvidedDelegation {
                    outer_role: Some(o),
                    inner_role,
                    ..
                } if *m.roles[*o].id == *rid => *inner_role,
                _ => None,
            })
    }

    /// The callbacks of event `ev` (before its handlers).
    #[cold]
    #[inline(never)]
    pub(super) fn listen(&mut self, ev: &Ev) {
        let t = self.now;
        let mut calls: Vec<Call> = Vec::new();
        match ev {
            Ev::UserStarted(c) => {
                let id = c.user.id;
                if self.lis.users.insert(id) {
                    let scenario = c.scenario as u32;
                    let kind = if c.closed {
                        ProcessKind::ClosedUser { scenario }
                    } else {
                        ProcessKind::OpenUser { scenario }
                    };
                    calls.push(Call::Created(id, None, kind));
                }
                calls.push(Call::Scenario(true, id, c.scenario as u32));
            }
            Ev::UserFinished(c) => {
                let id = c.user.id;
                calls.push(Call::Scenario(false, id, c.scenario as u32));
                if !c.closed && self.lis.users.remove(&id) {
                    calls.push(Call::Ended(id));
                }
            }
            Ev::UserAborted(c) => {
                if self.lis.users.remove(&c.user.id) {
                    calls.push(Call::Ended(c.user.id));
                }
            }
            Ev::EntryReq(r, _) | Ev::ReqFinished(r, _) => {
                let begin = matches!(ev, Ev::EntryReq(..));
                let id = r.user.id;
                let sys = r.role.zip(r.sig);
                if begin {
                    calls.push(Call::Entry(true, id, r.uact));
                    if let Some((role, sig)) = sys {
                        calls.push(Call::SysOp(true, id, role, sig));
                    }
                } else {
                    if let Some((role, sig)) = sys {
                        calls.push(Call::SysOp(false, id, role, sig));
                    }
                    calls.push(Call::Entry(false, id, r.uact));
                }
            }
            Ev::ExtCalled(g) => {
                if let GerKind::External(a) = g.kind {
                    calls.push(Call::Ext(true, self.listen_pid(&g.from), a));
                }
            }
            Ev::CowOk(cow) if cow.reply => {
                if let GerKind::External(a) = cow.entry.kind {
                    calls.push(Call::Ext(false, self.listen_pid(&cow.entry.from), a));
                }
            }
            Ev::SProgressed(c) => {
                // the first step of a provider's SEFF: its assembly operation begins
                let (root, key) = {
                    let h = c.holder.borrow();
                    (matches!(h.kind, HKind::Root) && !h.child, hid(&c.holder))
                };
                if root
                    && c.parent.is_none()
                    && !self.lis.ops.contains_key(&key)
                    && let Some(sig) = match (&c.cow, &c.rpc.request) {
                        (Some(cow), _) => cow.entry.sig,
                        (None, Some(r)) => r.sig,
                        _ => None,
                    }
                {
                    let role = match (&c.cow, &c.rpc.request) {
                        (Some(cow), _) => self
                            .assembly_connector(cow.entry.role)
                            .ok()
                            .flatten()
                            .and_then(|x| x.1),
                        (None, Some(r)) => self.listen_inner_role(r.role),
                        _ => None,
                    };
                    if let Some(role) = role {
                        let op = AssemblyOp {
                            assembly: c.ac,
                            role,
                            signature: sig,
                            container: self.alloc.get(c.ac.index()).copied().flatten(),
                        };
                        let p = self.listen_pid(c);
                        self.lis.ops.insert(key, (op, p));
                        calls.push(Call::Op(true, p, op));
                    }
                }
            }
            Ev::SChild(c) => {
                let (fork, key, beh) = {
                    let h = c.holder.borrow();
                    (
                        matches!(h.kind, HKind::Fork { .. }),
                        hid(&c.holder),
                        h.wrappers.first().map(|w| w.beh),
                    )
                };
                if fork
                    && !self.lis.forks.contains_key(&key)
                    && let Some(beh) = beh
                    && let BehaviourOwner::ForkedSync(a) | BehaviourOwner::ForkedAsync(a) =
                        self.m.behaviours[beh].owner
                {
                    let parent = c.parent.as_deref().map(|p| self.listen_pid(p));
                    let p = FORK_PID | c.id;
                    self.lis.forks.insert(key, p);
                    calls.push(Call::Created(
                        p,
                        parent,
                        ProcessKind::Forked {
                            fork: a,
                            behaviour: beh,
                            synchronous: true,
                        },
                    ));
                }
            }
            Ev::SFinished(c) => {
                let key = hid(&c.holder);
                let (kind_fork, done, child, processed) = {
                    let h = c.holder.borrow();
                    let processed = matches!(
                        h.kind,
                        HKind::Fork {
                            processed: true,
                            ..
                        }
                    );
                    (
                        matches!(h.kind, HKind::Fork { .. }),
                        Self::has_finished(&h),
                        h.child,
                        processed,
                    )
                };
                if kind_fork {
                    // the fork's behaviours are done: its process ends
                    if done
                        && !processed
                        && let Some(p) = self.lis.forks.remove(&key)
                    {
                        calls.push(Call::Ended(p));
                    }
                } else if done
                    && !child
                    && let Some((op, p)) = self.lis.ops.remove(&key)
                {
                    calls.push(Call::Op(false, p, op));
                }
            }
            _ => {}
        }
        let Some(l) = self.lis.l.as_deref_mut() else {
            return;
        };
        for c in calls {
            match c {
                Call::Created(p, parent, k) => l.process_created(t, p, parent, k),
                Call::Ended(p) => l.process_ended(t, p),
                Call::Scenario(b, p, s) => l.usage_scenario(t, b, p, s),
                Call::Entry(b, p, a) => l.entry_call(t, b, p, a),
                Call::SysOp(b, p, r, s) => l.system_operation(t, b, p, r, s),
                Call::Ext(b, p, a) => l.external_call(t, b, p, a),
                Call::Op(b, p, op) => l.assembly_operation(t, b, p, &op),
            }
        }
    }
}

enum Call {
    Created(u64, Option<u64>, ProcessKind),
    Ended(u64),
    Scenario(bool, u64, u32),
    Entry(bool, u64, UserActionId),
    SysOp(bool, u64, RoleId, SignatureId),
    Ext(bool, u64, ActionId),
    Op(bool, u64, AssemblyOp),
}
