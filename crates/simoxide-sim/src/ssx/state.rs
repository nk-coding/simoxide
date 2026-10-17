//! The state of a paused Slingshot-exact run as plain data ([`SsxState`]; `DESIGN-SPD.md` 5.7,
//! "ssx snapshots as built"): forks, detached runs (another thread) and snapshots on disk
//! (section `ssx`, `docs/formats/snapshot.md`).
//!
//! Slingshot's entities are `Rc` objects shared between events and partly mutable through
//! every reference (a user's SimuCom stack of frames, a fork holder's iterator, a job's
//! demand), so the state keeps them as one **object arena**: every object reachable from the
//! event list, the jobs at a resource and the passive resources' waiting queues gets one node,
//! written after the nodes it refers to (depth first, post order, without recursion), and
//! references are node indices. Rebuilding the nodes in order gives one object per node: the
//! sharing, and so the identity semantics, are kept. The nodes come in a fixed order (the event
//! list by insertion number, the jobs by id, the queues in order), so the same state gives the
//! same arena, and a restored state writes the bytes it was read from.
//!
//! The rest is plain data: the schedulers, both random streams with their positions, the
//! projection's numbering maps and line count, the monitors' calculators and keys (maps in key
//! order), the disposed bits, passive resources, `res_live` and the linking-resource table; with
//! SPD the engine's side of the layer (time events, subscribers, their disposed bits) and the
//! run's own model (the last compiled model with its build order and generation, and the graph
//! override that carries a QVTo host's state). The aliases of created elements and the ids of
//! the input elements are recomputed from the models.

use super::*;
use crate::frames::{Frame, FrameRef, Proxy, ProxyRef};
use simoxide_model::raw::{Graph, ObjId};
use std::collections::HashMap as Map;
use std::sync::Arc;

#[cfg(feature = "snapshot")]
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------------------------------
// the object arena

/// A StoEx value in a frame.
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub(crate) enum ValueImg {
    Int(i32),
    Double(f64),
    Bool(bool),
    Str(String),
    Null,
}

impl From<&Value> for ValueImg {
    fn from(v: &Value) -> Self {
        match v {
            Value::Int(i) => ValueImg::Int(*i),
            Value::Double(d) => ValueImg::Double(*d),
            Value::Bool(b) => ValueImg::Bool(*b),
            Value::Str(s) => ValueImg::Str(s.to_string()),
            Value::Null => ValueImg::Null,
        }
    }
}

impl From<ValueImg> for Value {
    fn from(v: ValueImg) -> Self {
        match v {
            ValueImg::Int(i) => Value::Int(i),
            ValueImg::Double(d) => Value::Double(d),
            ValueImg::Bool(b) => Value::Bool(b),
            ValueImg::Str(s) => Value::Str(s.into()),
            ValueImg::Null => Value::Null,
        }
    }
}

/// A frame entry: a value, or an evaluation proxy (node index).
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub(crate) enum BindImg {
    Val(ValueImg),
    Proxy(u32),
}

/// A job's kind with node references.
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub(crate) enum JobKindImg {
    Active {
        rtype: Option<ResourceTypeId>,
        req: u32,
        container: Option<ContainerId>,
    },
    Link {
        link: LinkingResourceId,
        cow: u32,
    },
}

/// A node of the object arena: references are indices of earlier nodes.
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub(crate) enum NodeImg {
    /// An immutable frame (`copyFrame` of an `INNER` proxy).
    IFrame {
        entries: Vec<(KeyId, BindImg)>,
        parent: Option<u32>,
    },
    /// An evaluation proxy.
    Proxy {
        prog: ProgId,
        frame: Option<u32>,
    },
    /// A shared mutable SimuCom frame.
    SFrame {
        entries: Vec<(KeyId, BindImg)>,
        parent: Option<u32>,
    },
    User {
        id: u64,
        stack: Vec<u32>,
    },
    UBeh {
        kind: UBehKind,
        behaviour: ScenarioBehaviourId,
        next: Option<Option<UserActionId>>,
        parent: Option<u32>,
        start_once: bool,
        max: i32,
        progression: i32,
    },
    UCtx {
        scenario: usize,
        action: Option<UserActionId>,
        user: u32,
        beh: u32,
        result: Option<u32>,
        closed: bool,
    },
    UReq {
        user: u32,
        uact: UserActionId,
        role: Option<RoleId>,
        sig: Option<SignatureId>,
    },
    Holder {
        kind: HKind,
        wrappers: Vec<Wrapper>,
        child: bool,
    },
    Rpc {
        role: Option<RoleId>,
        user: u32,
        uctx: Option<u32>,
        request: Option<u32>,
    },
    Seff {
        id: u64,
        holder: u32,
        rpc: u32,
        ac: AssemblyContextId,
        caller: Option<u32>,
        cow: Option<u32>,
        parent: Option<u32>,
        result: Option<u32>,
    },
    Ger {
        kind: GerKind,
        role: Option<RoleId>,
        sig: Option<SignatureId>,
        user: u32,
        from: u32,
    },
    Cow {
        from: AssemblyContextId,
        to: AssemblyContextId,
        user: u32,
        entry: u32,
        vars: Option<u32>,
        reply: bool,
    },
    Rdr {
        ac: AssemblyContextId,
        ctx: u32,
        passive: Option<PassiveResourceId>,
        active: bool,
        action: ActionId,
        demand: usize,
    },
    Job {
        id: u64,
        demand: f64,
        kind: JobKindImg,
    },
    Adjusted(spd::Adjusted),
}

/// An object of the engine (an `Rc`).
#[derive(Clone)]
enum Obj {
    IFrame(FrameRef),
    Proxy(ProxyRef),
    SFrame(SFrame),
    User(URef),
    UBeh(Rc<UBeh>),
    UCtx(Rc<UCtx>),
    UReq(Rc<UserRequest>),
    Holder(HRef),
    Rpc(Rc<Rpc>),
    Seff(Rc<SeffCtx>),
    Ger(Rc<Ger>),
    Cow(Rc<Cow>),
    Rdr(Rc<Rdr>),
    Job(Rc<Job>),
    Adjusted(Rc<spd::Adjusted>),
}

fn addr<T: ?Sized>(r: &Rc<T>) -> usize {
    Rc::as_ptr(r) as *const () as usize
}

impl Obj {
    /// Identity: kind and address.
    fn key(&self) -> (u8, usize) {
        match self {
            Obj::IFrame(x) => (0, addr(x)),
            Obj::Proxy(x) => (1, addr(x)),
            Obj::SFrame(x) => (2, addr(x)),
            Obj::User(x) => (3, addr(x)),
            Obj::UBeh(x) => (4, addr(x)),
            Obj::UCtx(x) => (5, addr(x)),
            Obj::UReq(x) => (6, addr(x)),
            Obj::Holder(x) => (7, addr(x)),
            Obj::Rpc(x) => (8, addr(x)),
            Obj::Seff(x) => (9, addr(x)),
            Obj::Ger(x) => (10, addr(x)),
            Obj::Cow(x) => (11, addr(x)),
            Obj::Rdr(x) => (12, addr(x)),
            Obj::Job(x) => (13, addr(x)),
            Obj::Adjusted(x) => (14, addr(x)),
        }
    }

    /// The objects this one refers to, in field order.
    fn deps(&self) -> Vec<Obj> {
        fn proxies(es: &[(KeyId, Binding)], out: &mut Vec<Obj>) {
            for (_, b) in es {
                if let Binding::Proxy(p) = b {
                    out.push(Obj::Proxy(p.clone()));
                }
            }
        }
        let mut v = Vec::new();
        match self {
            Obj::IFrame(f) => {
                v.extend(f.parent.clone().map(Obj::IFrame));
                proxies(&f.entries, &mut v);
            }
            Obj::Proxy(p) => v.extend(p.frame.clone().map(Obj::IFrame)),
            Obj::SFrame(f) => {
                let f = f.borrow();
                v.extend(f.parent.clone().map(Obj::SFrame));
                proxies(&f.entries, &mut v);
            }
            Obj::User(u) => v.extend(u.stack.borrow().iter().cloned().map(Obj::SFrame)),
            Obj::UBeh(b) => v.extend(b.parent.clone().map(Obj::UBeh)),
            Obj::UCtx(c) => {
                v.push(Obj::User(c.user.clone()));
                v.push(Obj::UBeh(c.beh.clone()));
                v.extend(c.result.clone().map(Obj::SFrame));
            }
            Obj::UReq(r) => v.push(Obj::User(r.user.clone())),
            Obj::Holder(_) | Obj::Adjusted(_) => {}
            Obj::Rpc(r) => {
                v.push(Obj::User(r.user.clone()));
                v.extend(r.uctx.clone().map(Obj::UCtx));
                v.extend(r.request.clone().map(Obj::UReq));
            }
            Obj::Seff(c) => {
                v.push(Obj::Holder(c.holder.clone()));
                v.push(Obj::Rpc(c.rpc.clone()));
                v.extend(c.caller.clone().map(Obj::Seff));
                v.extend(c.cow.clone().map(Obj::Cow));
                v.extend(c.parent.clone().map(Obj::Seff));
                v.extend(c.result.clone().map(Obj::SFrame));
            }
            Obj::Ger(g) => {
                v.push(Obj::User(g.user.clone()));
                v.push(Obj::Seff(g.from.clone()));
            }
            Obj::Cow(c) => {
                v.push(Obj::User(c.user.clone()));
                v.push(Obj::Ger(c.entry.clone()));
                v.extend(c.vars.clone().map(Obj::SFrame));
            }
            Obj::Rdr(r) => v.push(Obj::Seff(r.ctx.clone())),
            Obj::Job(j) => match &j.kind {
                JobKind::Active { req, .. } => v.push(Obj::Rdr(req.clone())),
                JobKind::Link { cow, .. } => v.push(Obj::Cow(cow.clone())),
            },
        }
        v
    }
}

/// Collects objects into the arena.
#[derive(Default)]
struct Writer {
    nodes: Vec<NodeImg>,
    ids: Map<(u8, usize), u32>,
}

impl Writer {
    /// The node of `root` (written with everything it refers to, if new).
    fn id(&mut self, root: Obj) -> u32 {
        let rk = root.key();
        if let Some(&i) = self.ids.get(&rk) {
            return i;
        }
        let mut stack = vec![(root, false)];
        while let Some((o, ready)) = stack.pop() {
            let k = o.key();
            if self.ids.contains_key(&k) {
                continue;
            }
            if !ready {
                let deps = o.deps();
                stack.push((o, true));
                for d in deps.into_iter().rev() {
                    if !self.ids.contains_key(&d.key()) {
                        stack.push((d, false));
                    }
                }
                continue;
            }
            let n = self.emit(&o);
            self.ids.insert(k, self.nodes.len() as u32);
            self.nodes.push(n);
        }
        self.ids[&rk]
    }

    /// The node of an object already written.
    fn of(&self, o: Obj) -> u32 {
        self.ids[&o.key()]
    }

    fn entries(&self, es: &[(KeyId, Binding)]) -> Vec<(KeyId, BindImg)> {
        es.iter()
            .map(|(k, b)| {
                let b = match b {
                    Binding::Val(v) => BindImg::Val(v.into()),
                    Binding::Proxy(p) => BindImg::Proxy(self.of(Obj::Proxy(p.clone()))),
                };
                (*k, b)
            })
            .collect()
    }

    /// The node of `o`, whose references are written.
    fn emit(&self, o: &Obj) -> NodeImg {
        let sf = |f: &Option<SFrame>| f.clone().map(|f| self.of(Obj::SFrame(f)));
        match o {
            Obj::IFrame(f) => NodeImg::IFrame {
                entries: self.entries(&f.entries),
                parent: f.parent.clone().map(|p| self.of(Obj::IFrame(p))),
            },
            Obj::Proxy(p) => NodeImg::Proxy {
                prog: p.prog,
                frame: p.frame.clone().map(|f| self.of(Obj::IFrame(f))),
            },
            Obj::SFrame(f) => {
                let f = f.borrow();
                NodeImg::SFrame {
                    entries: self.entries(&f.entries),
                    parent: sf(&f.parent),
                }
            }
            Obj::User(u) => NodeImg::User {
                id: u.id,
                stack: u
                    .stack
                    .borrow()
                    .iter()
                    .map(|f| self.of(Obj::SFrame(f.clone())))
                    .collect(),
            },
            Obj::UBeh(b) => NodeImg::UBeh {
                kind: b.kind,
                behaviour: b.behaviour,
                next: b.next,
                parent: b.parent.clone().map(|p| self.of(Obj::UBeh(p))),
                start_once: b.start_once.get(),
                max: b.max,
                progression: b.progression.get(),
            },
            Obj::UCtx(c) => NodeImg::UCtx {
                scenario: c.scenario,
                action: c.action,
                user: self.of(Obj::User(c.user.clone())),
                beh: self.of(Obj::UBeh(c.beh.clone())),
                result: sf(&c.result),
                closed: c.closed,
            },
            Obj::UReq(r) => NodeImg::UReq {
                user: self.of(Obj::User(r.user.clone())),
                uact: r.uact,
                role: r.role,
                sig: r.sig,
            },
            Obj::Holder(h) => {
                let h = h.borrow();
                NodeImg::Holder {
                    kind: h.kind.clone(),
                    wrappers: h.wrappers.clone(),
                    child: h.child,
                }
            }
            Obj::Rpc(r) => NodeImg::Rpc {
                role: r.role,
                user: self.of(Obj::User(r.user.clone())),
                uctx: r.uctx.clone().map(|x| self.of(Obj::UCtx(x))),
                request: r.request.clone().map(|x| self.of(Obj::UReq(x))),
            },
            Obj::Seff(c) => NodeImg::Seff {
                id: c.id,
                holder: self.of(Obj::Holder(c.holder.clone())),
                rpc: self.of(Obj::Rpc(c.rpc.clone())),
                ac: c.ac,
                caller: c.caller.clone().map(|x| self.of(Obj::Seff(x))),
                cow: c.cow.clone().map(|x| self.of(Obj::Cow(x))),
                parent: c.parent.clone().map(|x| self.of(Obj::Seff(x))),
                result: sf(&c.result),
            },
            Obj::Ger(g) => NodeImg::Ger {
                kind: g.kind,
                role: g.role,
                sig: g.sig,
                user: self.of(Obj::User(g.user.clone())),
                from: self.of(Obj::Seff(g.from.clone())),
            },
            Obj::Cow(c) => NodeImg::Cow {
                from: c.from,
                to: c.to,
                user: self.of(Obj::User(c.user.clone())),
                entry: self.of(Obj::Ger(c.entry.clone())),
                vars: sf(&c.vars),
                reply: c.reply,
            },
            Obj::Rdr(r) => NodeImg::Rdr {
                ac: r.ac,
                ctx: self.of(Obj::Seff(r.ctx.clone())),
                passive: r.passive,
                active: r.active,
                action: r.action,
                demand: r.demand,
            },
            Obj::Job(j) => NodeImg::Job {
                id: j.id,
                demand: j.demand.get(),
                kind: match &j.kind {
                    JobKind::Active {
                        rtype,
                        req,
                        container,
                    } => JobKindImg::Active {
                        rtype: *rtype,
                        req: self.of(Obj::Rdr(req.clone())),
                        container: *container,
                    },
                    JobKind::Link { link, cow } => JobKindImg::Link {
                        link: *link,
                        cow: self.of(Obj::Cow(cow.clone())),
                    },
                },
            },
            Obj::Adjusted(a) => NodeImg::Adjusted((**a).clone()),
        }
    }
}

/// The objects of an arena, rebuilt in node order.
struct Rebuilt {
    objs: Vec<Obj>,
}

const BAD: &str = "damaged object arena";

macro_rules! getter {
    ($name:ident, $variant:ident, $t:ty) => {
        fn $name(&self, i: u32) -> Result<$t, String> {
            match self.objs.get(i as usize) {
                Some(Obj::$variant(x)) => Ok(x.clone()),
                _ => Err(BAD.into()),
            }
        }
    };
}

impl Rebuilt {
    getter!(iframe, IFrame, FrameRef);
    getter!(proxy, Proxy, ProxyRef);
    getter!(sframe, SFrame, SFrame);
    getter!(user, User, URef);
    getter!(ubeh, UBeh, Rc<UBeh>);
    getter!(uctx, UCtx, Rc<UCtx>);
    getter!(ureq, UReq, Rc<UserRequest>);
    getter!(holder, Holder, HRef);
    getter!(rpc, Rpc, Rc<Rpc>);
    getter!(seff, Seff, Rc<SeffCtx>);
    getter!(ger, Ger, Rc<Ger>);
    getter!(cow, Cow, Rc<Cow>);
    getter!(rdr, Rdr, Rc<Rdr>);
    getter!(job, Job, Rc<Job>);
    getter!(adjusted, Adjusted, Rc<spd::Adjusted>);

    fn opt<T>(
        &self,
        i: Option<u32>,
        f: impl Fn(&Self, u32) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        i.map(|i| f(self, i)).transpose()
    }

    fn entries(&self, es: Vec<(KeyId, BindImg)>) -> Result<Vec<(KeyId, Binding)>, String> {
        es.into_iter()
            .map(|(k, b)| {
                Ok((
                    k,
                    match b {
                        BindImg::Val(v) => Binding::Val(v.into()),
                        BindImg::Proxy(i) => Binding::Proxy(self.proxy(i)?),
                    },
                ))
            })
            .collect()
    }

    fn new(nodes: Vec<NodeImg>) -> Result<Rebuilt, String> {
        let mut r = Rebuilt {
            objs: Vec::with_capacity(nodes.len()),
        };
        for n in nodes {
            // every reference is to an earlier node (checked by the getters' bounds)
            let o = match n {
                NodeImg::IFrame { entries, parent } => Obj::IFrame(FrameRef::new(Frame {
                    entries: r.entries(entries)?,
                    parent: r.opt(parent, Self::iframe)?,
                })),
                NodeImg::Proxy { prog, frame } => Obj::Proxy(ProxyRef::new(Proxy {
                    prog,
                    frame: r.opt(frame, Self::iframe)?,
                })),
                NodeImg::SFrame { entries, parent } => {
                    Obj::SFrame(Rc::new(RefCell::new(frame::JFrame {
                        entries: r.entries(entries)?,
                        parent: r.opt(parent, Self::sframe)?,
                    })))
                }
                NodeImg::User { id, stack } => Obj::User(Rc::new(User {
                    id,
                    stack: RefCell::new(
                        stack
                            .into_iter()
                            .map(|i| r.sframe(i))
                            .collect::<Result<_, _>>()?,
                    ),
                })),
                NodeImg::UBeh {
                    kind,
                    behaviour,
                    next,
                    parent,
                    start_once,
                    max,
                    progression,
                } => Obj::UBeh(Rc::new(UBeh {
                    kind,
                    behaviour,
                    next,
                    parent: r.opt(parent, Self::ubeh)?,
                    start_once: Cell::new(start_once),
                    max,
                    progression: Cell::new(progression),
                })),
                NodeImg::UCtx {
                    scenario,
                    action,
                    user,
                    beh,
                    result,
                    closed,
                } => Obj::UCtx(Rc::new(UCtx {
                    scenario,
                    action,
                    user: r.user(user)?,
                    beh: r.ubeh(beh)?,
                    result: r.opt(result, Self::sframe)?,
                    closed,
                })),
                NodeImg::UReq {
                    user,
                    uact,
                    role,
                    sig,
                } => Obj::UReq(Rc::new(UserRequest {
                    user: r.user(user)?,
                    uact,
                    role,
                    sig,
                })),
                NodeImg::Holder {
                    kind,
                    wrappers,
                    child,
                } => Obj::Holder(Rc::new(RefCell::new(Holder {
                    kind,
                    wrappers,
                    child,
                }))),
                NodeImg::Rpc {
                    role,
                    user,
                    uctx,
                    request,
                } => Obj::Rpc(Rc::new(Rpc {
                    role,
                    user: r.user(user)?,
                    uctx: r.opt(uctx, Self::uctx)?,
                    request: r.opt(request, Self::ureq)?,
                })),
                NodeImg::Seff {
                    id,
                    holder,
                    rpc,
                    ac,
                    caller,
                    cow,
                    parent,
                    result,
                } => Obj::Seff(Rc::new(SeffCtx {
                    id,
                    holder: r.holder(holder)?,
                    rpc: r.rpc(rpc)?,
                    ac,
                    caller: r.opt(caller, Self::seff)?,
                    cow: r.opt(cow, Self::cow)?,
                    parent: r.opt(parent, Self::seff)?,
                    result: r.opt(result, Self::sframe)?,
                })),
                NodeImg::Ger {
                    kind,
                    role,
                    sig,
                    user,
                    from,
                } => Obj::Ger(Rc::new(Ger {
                    kind,
                    role,
                    sig,
                    user: r.user(user)?,
                    from: r.seff(from)?,
                })),
                NodeImg::Cow {
                    from,
                    to,
                    user,
                    entry,
                    vars,
                    reply,
                } => Obj::Cow(Rc::new(Cow {
                    from,
                    to,
                    user: r.user(user)?,
                    entry: r.ger(entry)?,
                    vars: r.opt(vars, Self::sframe)?,
                    reply,
                })),
                NodeImg::Rdr {
                    ac,
                    ctx,
                    passive,
                    active,
                    action,
                    demand,
                } => Obj::Rdr(Rc::new(Rdr {
                    ac,
                    ctx: r.seff(ctx)?,
                    passive,
                    active,
                    action,
                    demand,
                })),
                NodeImg::Job { id, demand, kind } => Obj::Job(Rc::new(Job {
                    id,
                    demand: Cell::new(demand),
                    kind: match kind {
                        JobKindImg::Active {
                            rtype,
                            req,
                            container,
                        } => JobKind::Active {
                            rtype,
                            req: r.rdr(req)?,
                            container,
                        },
                        JobKindImg::Link { link, cow } => JobKind::Link {
                            link,
                            cow: r.cow(cow)?,
                        },
                    },
                })),
                NodeImg::Adjusted(a) => Obj::Adjusted(Rc::new(a)),
            };
            r.objs.push(o);
        }
        Ok(r)
    }
}

// ------------------------------------------------------------------------------------------
// events

/// An event with node references.
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub(crate) enum EvImg {
    PreSim,
    SimStarted,
    Finished,
    UPassed(u32, UserActionId),
    UserStarted(u32),
    InterArrival(u32),
    WokeUp(u32),
    ReqFinished(u32, u32),
    UserFinished(u32),
    UserAborted(u32),
    ClosedInit(u32),
    InnerInit(u32),
    UProgressed(u32),
    EntryReq(u32, u32),
    SProgressed(u32),
    SChild(u32),
    SFinished(u32),
    SPassed(u32, ActionId),
    ExtCalled(u32),
    InfraCalled(u32),
    CowReq(u32),
    CowOk(u32),
    CowAbort(u32),
    RdReq(u32),
    RdAbort(u32),
    ArFinished(u32),
    JobInit(u32),
    JobProg(u32, Wakeup<u64>),
    JobFin(u32),
    JobAbort(u32),
    ArState(ResIdx, u64, f64),
    ArDemand(ResIdx, f64),
    PAcq(u32),
    PRel(u32),
    PState(u32, i64),
    Meas(SeriesId, f64, f64),
    SpdTime(u32),
    SpdReq(u32, String),
    ModelAdjusted(u32),
}

fn ev_img(w: &mut Writer, ev: &Ev) -> EvImg {
    use EvImg as I;
    let mut uc = |c: &Rc<UCtx>| w.id(Obj::UCtx(c.clone()));
    match ev {
        Ev::PreSim => I::PreSim,
        Ev::SimStarted => I::SimStarted,
        Ev::Finished => I::Finished,
        Ev::UPassed(c, a) => I::UPassed(uc(c), *a),
        Ev::UserStarted(c) => I::UserStarted(uc(c)),
        Ev::InterArrival(c) => I::InterArrival(uc(c)),
        Ev::WokeUp(c) => I::WokeUp(uc(c)),
        Ev::UserFinished(c) => I::UserFinished(uc(c)),
        Ev::UserAborted(c) => I::UserAborted(uc(c)),
        Ev::ClosedInit(c) => I::ClosedInit(uc(c)),
        Ev::InnerInit(c) => I::InnerInit(uc(c)),
        Ev::UProgressed(c) => I::UProgressed(uc(c)),
        Ev::ReqFinished(r, c) => {
            let r = w.id(Obj::UReq(r.clone()));
            I::ReqFinished(r, w.id(Obj::UCtx(c.clone())))
        }
        Ev::EntryReq(r, c) => {
            let r = w.id(Obj::UReq(r.clone()));
            I::EntryReq(r, w.id(Obj::UCtx(c.clone())))
        }
        Ev::SProgressed(c) => I::SProgressed(w.id(Obj::Seff(c.clone()))),
        Ev::SChild(c) => I::SChild(w.id(Obj::Seff(c.clone()))),
        Ev::SFinished(c) => I::SFinished(w.id(Obj::Seff(c.clone()))),
        Ev::SPassed(c, a) => I::SPassed(w.id(Obj::Seff(c.clone())), *a),
        Ev::ExtCalled(g) => I::ExtCalled(w.id(Obj::Ger(g.clone()))),
        Ev::InfraCalled(g) => I::InfraCalled(w.id(Obj::Ger(g.clone()))),
        Ev::CowReq(c) => I::CowReq(w.id(Obj::Cow(c.clone()))),
        Ev::CowOk(c) => I::CowOk(w.id(Obj::Cow(c.clone()))),
        Ev::CowAbort(c) => I::CowAbort(w.id(Obj::Cow(c.clone()))),
        Ev::RdReq(r) => I::RdReq(w.id(Obj::Rdr(r.clone()))),
        Ev::RdAbort(r) => I::RdAbort(w.id(Obj::Rdr(r.clone()))),
        Ev::ArFinished(r) => I::ArFinished(w.id(Obj::Rdr(r.clone()))),
        Ev::JobInit(j) => I::JobInit(w.id(Obj::Job(j.clone()))),
        Ev::JobProg(j, x) => I::JobProg(w.id(Obj::Job(j.clone())), *x),
        Ev::JobFin(j) => I::JobFin(w.id(Obj::Job(j.clone()))),
        Ev::JobAbort(j) => I::JobAbort(w.id(Obj::Job(j.clone()))),
        Ev::ArState(r, n, u) => I::ArState(*r, *n, *u),
        Ev::ArDemand(r, v) => I::ArDemand(*r, *v),
        Ev::PAcq(r) => I::PAcq(w.id(Obj::Rdr(r.clone()))),
        Ev::PRel(r) => I::PRel(w.id(Obj::Rdr(r.clone()))),
        Ev::PState(r, a) => I::PState(w.id(Obj::Rdr(r.clone())), *a),
        Ev::Meas(s, t, v) => I::Meas(*s, *t, *v),
        Ev::SpdTime(i) => I::SpdTime(*i),
        Ev::SpdReq(r, p) => I::SpdReq(*r, p.to_string()),
        Ev::ModelAdjusted(a) => I::ModelAdjusted(w.id(Obj::Adjusted(a.clone()))),
    }
}

fn ev_of(b: &Rebuilt, e: EvImg) -> Result<Ev, String> {
    use EvImg as I;
    Ok(match e {
        I::PreSim => Ev::PreSim,
        I::SimStarted => Ev::SimStarted,
        I::Finished => Ev::Finished,
        I::UPassed(c, a) => Ev::UPassed(b.uctx(c)?, a),
        I::UserStarted(c) => Ev::UserStarted(b.uctx(c)?),
        I::InterArrival(c) => Ev::InterArrival(b.uctx(c)?),
        I::WokeUp(c) => Ev::WokeUp(b.uctx(c)?),
        I::ReqFinished(r, c) => Ev::ReqFinished(b.ureq(r)?, b.uctx(c)?),
        I::UserFinished(c) => Ev::UserFinished(b.uctx(c)?),
        I::UserAborted(c) => Ev::UserAborted(b.uctx(c)?),
        I::ClosedInit(c) => Ev::ClosedInit(b.uctx(c)?),
        I::InnerInit(c) => Ev::InnerInit(b.uctx(c)?),
        I::UProgressed(c) => Ev::UProgressed(b.uctx(c)?),
        I::EntryReq(r, c) => Ev::EntryReq(b.ureq(r)?, b.uctx(c)?),
        I::SProgressed(c) => Ev::SProgressed(b.seff(c)?),
        I::SChild(c) => Ev::SChild(b.seff(c)?),
        I::SFinished(c) => Ev::SFinished(b.seff(c)?),
        I::SPassed(c, a) => Ev::SPassed(b.seff(c)?, a),
        I::ExtCalled(g) => Ev::ExtCalled(b.ger(g)?),
        I::InfraCalled(g) => Ev::InfraCalled(b.ger(g)?),
        I::CowReq(c) => Ev::CowReq(b.cow(c)?),
        I::CowOk(c) => Ev::CowOk(b.cow(c)?),
        I::CowAbort(c) => Ev::CowAbort(b.cow(c)?),
        I::RdReq(r) => Ev::RdReq(b.rdr(r)?),
        I::RdAbort(r) => Ev::RdAbort(b.rdr(r)?),
        I::ArFinished(r) => Ev::ArFinished(b.rdr(r)?),
        I::JobInit(j) => Ev::JobInit(b.job(j)?),
        I::JobProg(j, x) => Ev::JobProg(b.job(j)?, x),
        I::JobFin(j) => Ev::JobFin(b.job(j)?),
        I::JobAbort(j) => Ev::JobAbort(b.job(j)?),
        I::ArState(r, n, u) => Ev::ArState(r, n, u),
        I::ArDemand(r, v) => Ev::ArDemand(r, v),
        I::PAcq(r) => Ev::PAcq(b.rdr(r)?),
        I::PRel(r) => Ev::PRel(b.rdr(r)?),
        I::PState(r, a) => Ev::PState(b.rdr(r)?, a),
        I::Meas(s, t, v) => Ev::Meas(s, t, v),
        I::SpdTime(i) => Ev::SpdTime(i),
        I::SpdReq(r, p) => Ev::SpdReq(r, p.into()),
        I::ModelAdjusted(a) => Ev::ModelAdjusted(b.adjusted(a)?),
    })
}

// ------------------------------------------------------------------------------------------
// the engine's state

/// Everything of the engine except the run's own model and the configuration.
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub(crate) struct EngineImg {
    pub(crate) now: f64,
    seq: u64,
    /// The event list: `(time, insertion number, event)` by insertion number.
    entries: Vec<(f64, u64, EvImg)>,
    nodes: Vec<NodeImg>,
    // random streams (MT19937 words and position, draws, `Math.random()`), tape position
    mt: (Vec<u32>, u32),
    stoex: u64,
    math: crate::javarand::JavaRandom,
    pos: i64,
    disposed: Vec<bool>,
    errors: Vec<String>,
    meas: crate::Measurements,
    mon: Monitors,
    next_user: u64,
    next_ctx: u64,
    next_job: u64,
    // projection: written, its line count, users and jobs by first appearance
    pub(crate) traced: bool,
    lines: i64,
    users: Vec<(u64, u64)>,
    jobs: Vec<(u64, u64)>,
    // resources
    res: Vec<resource::ResRt>,
    res_live: Vec<bool>,
    links: Vec<LinkingResourceId>,
    /// Per passive resource: available units, waiting requests (node) with their amounts.
    passives: Vec<(i64, Vec<(u32, i64)>)>,
    passive_idx: Vec<((PassiveResourceId, AssemblyContextId), usize)>,
    /// Jobs at a resource: id, node.
    live: Vec<(u64, u32)>,
    alloc: Vec<Option<ContainerId>>,
    sys: StructureId,
    system: SystemId,
    // run
    stopped: bool,
    pub(crate) done: bool,
    pub(crate) events: u64,
    finished: i64,
    same_instant: u64,
    last_t: f64,
    failure: Option<SimError>,
    /// The error that ended the run.
    pub(crate) error: Option<SimError>,
    spd: spd::SpdSide,
    /// The run model's generation (0: the base model).
    pub(crate) generation: u32,
}

/// The run's own compiled model after model changes, with its build order.
#[derive(Clone)]
pub(crate) struct OwnModel {
    pub(crate) cm: Arc<CompiledModel>,
    pub(crate) order: Arc<[ObjId]>,
}

/// The state of a paused Slingshot-exact run as a value ([`SsxRun::detach`]): plain data
/// (`Send`), continued with [`SsxRun::attach`] on any thread, written to a snapshot with
/// [`SsxState::serialize`] (feature `snapshot`).
#[derive(Clone)]
pub struct SsxState {
    pub(crate) img: Box<EngineImg>,
    pub(crate) cfg: SimConfig,
    pub(crate) model: Option<OwnModel>,
    /// The graph override (`ChangeHost::set_model_graph`: a QVTo host's state).
    pub(crate) graph: Option<Arc<Graph>>,
}

fn sorted<K: Ord + Copy, V: Copy>(m: &HashMap<K, V>) -> Vec<(K, V)> {
    let mut v: Vec<(K, V)> = m.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_by_key(|a| a.0);
    v
}

impl SsxState {
    /// The state of `r` (between events; outputs flushed by the caller).
    pub(crate) fn of(r: &SsxRun<'_>) -> SsxState {
        let e = &r.e;
        let mut w = Writer::default();
        let mut heap: Vec<&Entry> = e.heap.iter().collect();
        heap.sort_by_key(|x| x.seq);
        let entries = heap
            .into_iter()
            .map(|x| (x.t, x.seq, ev_img(&mut w, &x.ev)))
            .collect();
        let mut live: Vec<(&u64, &Rc<Job>)> = e.live.iter().collect();
        live.sort_by_key(|x| *x.0);
        let live = live
            .into_iter()
            .map(|(id, j)| (*id, w.id(Obj::Job(j.clone()))))
            .collect();
        let passives = e
            .passives
            .iter()
            .map(|p| {
                (
                    p.available,
                    p.queue
                        .iter()
                        .map(|(r, n)| (w.id(Obj::Rdr(r.clone())), *n))
                        .collect(),
                )
            })
            .collect();
        let img = EngineImg {
            now: e.now,
            seq: e.seq,
            entries,
            nodes: std::mem::take(&mut w.nodes),
            mt: e.rng.mt_state(),
            stoex: e.rng.stoex,
            math: e.rng.math.clone(),
            pos: e.rng.pos,
            disposed: e.disposed.to_vec(),
            errors: e.errors.clone(),
            meas: e.meas.clone(),
            mon: e.mon.clone(),
            next_user: e.next_user,
            next_ctx: e.next_ctx,
            next_job: e.next_job,
            traced: e.proj.is_some(),
            lines: e.lines,
            users: sorted(&e.users),
            jobs: sorted(&e.jobs),
            res: e.res.clone(),
            res_live: e.res_live.clone(),
            links: e.links.clone(),
            passives,
            passive_idx: sorted(&e.passive_idx),
            live,
            alloc: e.alloc.clone(),
            sys: e.sys,
            system: e.system,
            stopped: e.stopped,
            done: e.done,
            events: e.events,
            finished: e.finished,
            same_instant: e.same_instant,
            last_t: e.last_t,
            failure: e.failure.clone(),
            error: r.error.clone(),
            spd: e.spd.clone(),
            generation: e.own.generation,
        };
        let model = match (e.own.models.last(), &e.own.order) {
            (Some(cm), Some(order)) if e.own.generation > 0 => Some(OwnModel {
                cm: cm.clone(),
                order: order.clone(),
            }),
            _ => None,
        };
        SsxState {
            img: Box::new(img),
            cfg: r.cfg.clone(),
            model,
            graph: e.own.graph.clone(),
        }
    }

    /// Simulation time (ns) of the last event.
    pub fn now_ns(&self) -> i64 {
        simoxide_sched::time::span(self.img.now)
    }

    /// Events dispatched so far.
    pub fn events(&self) -> u64 {
        self.img.events
    }

    /// The run model's generation.
    pub fn generation(&self) -> u32 {
        self.img.generation
    }

    /// The run continued on `base` with `out` (see [`SsxRun::attach`]).
    pub(crate) fn into_run<'m>(
        self,
        base: &'m CompiledModel,
        out: Outputs,
    ) -> Result<SsxRun<'m>, SimError> {
        let SsxState {
            img,
            cfg,
            model,
            graph,
        } = self;
        let img = *img;
        let fail = |message: String| SimError {
            message,
            at_ns: simoxide_sched::time::span(img.now),
            kind: SimErrorKind::Model,
        };
        if base.flavour.kind != FlavourKind::SlingshotExact {
            return Err(fail(format!(
                "the Slingshot-exact engine needs a model compiled for slingshot-exact, not {}",
                base.flavour.kind
            )));
        }
        if out.trace.is_some() && !img.traced {
            return Err(fail(
                "the run was created without a trace: its continuation cannot write one".into(),
            ));
        }
        let mut e = Engine::new(base, &cfg, &out)?;
        // the run's own model
        if let Some(own) = model {
            if img.generation == 0 {
                return Err(fail("a run model without model changes".into()));
            }
            // SAFETY: as in `Engine::apply_changes`: the `Arc` is kept in `e.own.models` until
            // the engine is dropped, so the pointee lives as long as the engine.
            e.cm = unsafe { &*Arc::as_ptr(&own.cm) };
            e.own.models.push(own.cm);
            e.own.order = Some(own.order);
            e.m = &e.cm.model;
            let input = e
                .own
                .input
                .get_or_insert_with(|| spd::ids_of(&base.model.graph));
            e.alias = spd::aliases(&e.cm.model.graph, input);
        } else if img.generation != 0 {
            return Err(fail("the run model of a changed run is missing".into()));
        }
        e.own.generation = img.generation;
        e.own.graph = graph;
        let cm = e.cm;
        // plausibility of the tables the state indexes into
        if img.res.len() > cm.resources.len()
            || img.res_live.len() != img.res.len()
            || img.disposed.len() != H_COUNT
            || img.alloc.len() != cm.model.assembly_contexts.len()
            || img.meas.series.len() != img.meas.rows.len()
        {
            return Err(fail("the run state does not fit the model's tables".into()));
        }
        let b = Rebuilt::new(img.nodes).map_err(fail)?;
        e.now = img.now;
        e.seq = img.seq;
        e.heap = img
            .entries
            .into_iter()
            .map(|(t, seq, ev)| {
                Ok(Entry {
                    t,
                    seq,
                    ev: ev_of(&b, ev)?,
                })
            })
            .collect::<Result<_, String>>()
            .map_err(fail)?;
        e.rng = SsRng::restore(&img.mt, img.stoex, img.math, img.pos, out.tape.is_some())
            .map_err(fail)?;
        e.disposed.copy_from_slice(&img.disposed);
        e.errors = img.errors;
        e.meas = img.meas;
        e.mon = img.mon;
        e.next_user = img.next_user;
        e.next_ctx = img.next_ctx;
        e.next_job = img.next_job;
        e.proj = img.traced.then(|| String::with_capacity(1 << 16));
        e.lines = img.lines;
        e.users = img.users.into_iter().collect();
        e.jobs = img.jobs.into_iter().collect();
        e.res = img.res;
        e.res_live = img.res_live;
        e.links = img.links;
        e.passives = img
            .passives
            .into_iter()
            .map(|(available, q)| {
                Ok(resource::Passive {
                    available,
                    queue: q
                        .into_iter()
                        .map(|(r, n)| Ok((b.rdr(r)?, n)))
                        .collect::<Result<_, String>>()?,
                })
            })
            .collect::<Result<_, String>>()
            .map_err(fail)?;
        e.passive_idx = img.passive_idx.into_iter().collect();
        e.live = img
            .live
            .into_iter()
            .map(|(id, j)| Ok((id, b.job(j)?)))
            .collect::<Result<_, String>>()
            .map_err(fail)?;
        e.alloc = img.alloc;
        e.sys = img.sys;
        e.system = img.system;
        e.stopped = img.stopped;
        e.done = img.done;
        e.events = img.events;
        e.finished = img.finished;
        e.same_instant = img.same_instant;
        e.last_t = img.last_t;
        e.failure = img.failure;
        e.spd = img.spd;
        Ok(SsxRun {
            e,
            cfg,
            out,
            error: img.error,
        })
    }
}

/// `Monitors::keys` in key order, metric names interned on read.
#[cfg(feature = "snapshot")]
pub(crate) mod keys_map {
    use super::*;
    use serde::{Deserializer, Serializer};

    type M = HashMap<(String, &'static str), SeriesId>;

    pub fn serialize<S: Serializer>(m: &M, s: S) -> Result<S::Ok, S::Error> {
        let mut v: Vec<(&str, &str, SeriesId)> =
            m.iter().map(|((k, n), x)| (k.as_str(), *n, *x)).collect();
        v.sort();
        v.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<M, D::Error> {
        let v = Vec::<(String, String, SeriesId)>::deserialize(d)?;
        Ok(v.into_iter()
            .map(|(k, n, x)| ((k, crate::snapshot::serde_util::intern(&n)), x))
            .collect())
    }
}

// ------------------------------------------------------------------------------------------
// snapshots on disk (format v1, section `ssx`)

/// Name of the Slingshot-exact engine's section.
#[cfg(feature = "snapshot")]
pub const SSX: &str = "ssx";

#[cfg(feature = "snapshot")]
mod disk {
    use super::*;
    use crate::sim::snapshot::{
        ConfigImage, graph_section, model_section, read_graph_section, read_model_section,
    };
    use crate::snapshot::{
        Encoding, FORMAT, Header, Section, Snapshot, SnapshotError, SnapshotInfo, SnapshotOptions,
        model_fingerprint,
    };

    /// The section `ssx` as written.
    #[derive(Serialize)]
    struct SsxOut<'a> {
        cfg: ConfigImage,
        engine: &'a EngineImg,
    }

    /// The section `ssx` as read.
    #[derive(Deserialize)]
    struct SsxIn {
        cfg: ConfigImage,
        engine: EngineImg,
    }

    impl SsxState {
        /// The snapshot of this state (the run was created on `base`): the sections `ssx`,
        /// after model changes `model`, with a graph override `graph`, encoded with `enc`.
        pub fn serialize(
            &self,
            base: &CompiledModel,
            enc: Encoding,
        ) -> Result<Snapshot, SnapshotError> {
            let img = &*self.img;
            let mut sections = vec![Section::encode(
                SSX,
                &SsxOut {
                    cfg: (&self.cfg).into(),
                    engine: img,
                },
                enc,
            )?];
            let mut run_model = None;
            if let Some(own) = &self.model {
                let (sec, info) = model_section(&own.cm, &own.order, img.generation, enc)?;
                sections.push(sec);
                run_model = Some(info);
            }
            if let Some(g) = &self.graph {
                sections.push(graph_section(g, enc)?);
            }
            let phase = match (&img.error, img.done, img.events) {
                (Some(_), _, _) => "failed",
                (None, true, _) => "finished",
                (None, false, 0) => "notstarted",
                (None, false, _) => "running",
            };
            let header = Header {
                format: FORMAT,
                simoxide: env!("CARGO_PKG_VERSION").to_string(),
                mode: self.cfg.mode.name().to_string(),
                literal: false,
                flavour: FlavourKind::SlingshotExact.name().to_string(),
                quirks: if base.flavour.quirks.is_empty() {
                    String::new()
                } else {
                    base.flavour.quirks.to_string()
                },
                model: model_fingerprint(base),
                run_model,
                time_ns: simoxide_sched::time::span(img.now),
                events: img.events,
                phase: phase.to_string(),
                traced: img.traced,
                encoding: enc,
                sections: Vec::new(),
            };
            Ok(Snapshot { header, sections })
        }

        /// The state of snapshot `s`, for a run created on `base` (which must be the compiled
        /// model the snapshot names; after model changes the state brings its own model).
        pub fn deserialize(s: &Snapshot, base: &CompiledModel) -> Result<SsxState, SnapshotError> {
            let h = &s.header;
            let this = env!("CARGO_PKG_VERSION");
            if h.simoxide != this {
                return Err(SnapshotError::SimOxideVersion {
                    found: h.simoxide.clone(),
                    this: this.to_string(),
                });
            }
            let name = FlavourKind::SlingshotExact.name();
            if h.flavour != name || base.flavour.kind != FlavourKind::SlingshotExact {
                return Err(SnapshotError::Mismatch(format!(
                    "the snapshot is of a {} flavour run; the Slingshot-exact engine runs {name} on a model compiled for {}",
                    h.flavour,
                    base.flavour.kind.name()
                )));
            }
            let fp = model_fingerprint(base);
            if fp != h.model {
                return Err(SnapshotError::Model {
                    expected: h.model.clone(),
                    found: fp,
                });
            }
            let sec = s
                .section(SSX)
                .ok_or_else(|| SnapshotError::Corrupt("no section `ssx`".into()))?;
            let x: SsxIn = sec.decode(h.encoding)?;
            let generation = x.engine.generation;
            let model = if generation == 0 {
                None
            } else {
                let (cm, order) = read_model_section(s, base, generation)?;
                Some(OwnModel {
                    cm: Arc::new(cm),
                    order: order.into(),
                })
            };
            let graph = read_graph_section(s)?.map(Arc::new);
            Ok(SsxState {
                img: Box::new(x.engine),
                cfg: x.cfg.into(),
                model,
                graph,
            })
        }
    }

    impl<'m> SsxRun<'m> {
        /// The snapshot of the run, between events (outputs flushed first): the sections of
        /// [`SsxState::serialize`] plus `extra` (sections of layers above the engine, encoded
        /// with the same encoding: `spd`, `qvto`, ...).
        pub fn snapshot_value(
            &mut self,
            enc: Encoding,
            extra: Vec<Section>,
        ) -> Result<Snapshot, SnapshotError> {
            let st = self.detach();
            let mut s = st.serialize(self.e.base, enc)?;
            s.sections.extend(extra);
            Ok(s)
        }

        /// Writes a snapshot of the run (between events) to `w` (postcard).
        /// [`SsxRun::restore_from`] continues it exactly like the uninterrupted run.
        pub fn snapshot_to(
            &mut self,
            w: impl std::io::Write,
        ) -> Result<SnapshotInfo, SnapshotError> {
            self.snapshot_with(w, &SnapshotOptions::default(), Vec::new())
        }

        /// [`SsxRun::snapshot_to`] with options and extra sections.
        pub fn snapshot_with(
            &mut self,
            mut w: impl std::io::Write,
            opts: &SnapshotOptions,
            extra: Vec<Section>,
        ) -> Result<SnapshotInfo, SnapshotError> {
            let s = self.snapshot_value(opts.encoding, extra)?;
            let mut bytes = Vec::new();
            let header = s.write(&mut bytes)?;
            w.write_all(&bytes)?;
            w.flush()?;
            Ok(SnapshotInfo {
                header,
                bytes: bytes.len() as u64,
            })
        }

        /// Restores a snapshot written by [`SsxRun::snapshot_to`]: the run continues on
        /// `base` (the compiled model it was created on; checked by fingerprint) with the
        /// outputs `out`, which receive the continuation only (see [`SsxRun::attach`]).
        pub fn restore_from(
            r: impl std::io::Read,
            base: &'m CompiledModel,
            out: Outputs,
        ) -> Result<Self, SnapshotError> {
            let s = Snapshot::read(r)?;
            Self::restore_snapshot(&s, base, out)
        }

        /// [`SsxRun::restore_from`] of a snapshot already read (layers above the engine read
        /// their own sections from it).
        pub fn restore_snapshot(
            s: &Snapshot,
            base: &'m CompiledModel,
            out: Outputs,
        ) -> Result<Self, SnapshotError> {
            let st = SsxState::deserialize(s, base)?;
            SsxRun::attach(base, st, out).map_err(|e| SnapshotError::Restore(e.message))
        }
    }
}
