//! Flat instruction streams of the behaviours, the form the interpreter executes.
//!
//! Every SEFF behaviour ([`CBeh`]) and every usage-scenario behaviour ([`CUBeh`]) is compiled
//! once per model into a straight sequence of small `Copy` instructions with pre-resolved
//! operands. An action becomes its `BEGIN` trace line, its effect and its `END` trace line;
//! an internal action's demands, infrastructure calls and resource calls become one instruction
//! each, and an external call (entry-level system call) a call and a return instruction around
//! the callee. The behaviour's continuation only holds a program counter: a wait (a demand, a
//! delay) or a child continuation (a call, a loop body) resumes at the next instruction.
//!
//! The instructions do exactly what the reference's interpreter does at these points, in the
//! same order (`docs/spec/actions.md`, `docs/spec/workloads.md`); only the number of
//! interpreter steps is smaller (see `Limits::max_steps`).

use crate::ir::{CAct, CBeh, CUAct, CUBeh, ProgId, SeriesId};
use simoxide_model::{ActionId, BehaviourId, ResourceTypeId, ScenarioBehaviourId, UserActionId};

/// No resource type (a demand whose resource type is unset).
pub const NO_TYPE: u32 = u32::MAX;

/// An instruction of a SEFF behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SOp {
    /// The `StartAction`: its `BEGIN` and `END` trace lines.
    Start(ActionId),
    /// Error: the behaviour has no start action.
    NoStart,
    /// `BEGIN` of an action.
    Begin(ActionId),
    /// `BEGIN` of an external call with a response-time series: also starts the measurement.
    BeginRt(ActionId, SeriesId),
    /// `END` of an action.
    End(ActionId),
    /// `END` of an external call with a response-time series: also records the measurement.
    EndRt(ActionId, SeriesId),
    /// A resource demand of an internal action (`rtype`: [`NO_TYPE`] if unset).
    Demand {
        a: ActionId,
        prog: ProgId,
        rtype: u32,
    },
    /// Infrastructure call `i` of an internal action (pushes its call loop).
    Infra(ActionId, u32),
    /// Resource call `i` of an internal action.
    ResCall(ActionId, u32),
    /// An external call: input frame, then the call of the required role (`site`: index of
    /// the call site, for the interpreter's cache of resolved calls).
    Call { a: ActionId, site: u32 },
    /// Return of an external call: output parameters (`returns`: it has any).
    Return { a: ActionId, returns: bool },
    /// Any other action (branch, loop, collection iterator, fork, acquire, release, set
    /// variable, recovery, unsupported).
    Exec(ActionId),
    /// The `StopAction` is reached: the behaviour ends.
    Ret,
    /// Error: the chain of successors ended without a `StopAction`.
    NoStop,
}

/// An instruction of a usage-scenario behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UOp {
    /// The `Start` action: its `BEGIN` and `END` trace lines.
    Start(UserActionId),
    /// Error: the behaviour has no start action.
    NoStart,
    /// `BEGIN` of a user action.
    Begin(UserActionId),
    /// `BEGIN` of an entry-level system call with a response-time series.
    BeginRt(UserActionId),
    /// `END` of a user action.
    End(UserActionId),
    /// `END` of an entry-level system call with a response-time series.
    EndRt(UserActionId, SeriesId),
    /// A delay.
    Delay(UserActionId, ProgId),
    /// An entry-level system call: input frame, then the call of the system's provided role.
    Call(UserActionId),
    /// Return of an entry-level system call.
    Return(UserActionId),
    /// A branch or a loop.
    Exec(UserActionId),
    /// The `Stop` action is reached.
    Ret,
    /// Error: the chain of successors ended without a `Stop` action.
    NoStop,
}

/// The compiled instruction streams of a model, in two variants: with the trace instructions
/// and without them (a run without a trace skips the `BEGIN`/`END` lines entirely).
#[derive(Debug, Clone, Default)]
pub struct Code {
    pub traced: Flat,
    pub plain: Flat,
}

impl Code {
    pub fn build(beh: &[CBeh], act: &[CAct], ubeh: &[CUBeh], uact: &[CUAct]) -> Code {
        Code {
            traced: Flat::build(beh, act, ubeh, uact, true),
            plain: Flat::build(beh, act, ubeh, uact, false),
        }
    }

    /// The variant for a run with (`true`) or without a trace.
    #[inline]
    pub fn get(&self, trace: bool) -> &Flat {
        if trace { &self.traced } else { &self.plain }
    }
}

/// One variant of the instruction streams.
#[derive(Debug, Clone, Default)]
pub struct Flat {
    /// Instructions of all SEFF behaviours.
    pub sops: Vec<SOp>,
    /// First instruction of each SEFF behaviour (by [`BehaviourId`]).
    pub sbeh: Vec<u32>,
    /// Instructions of all usage-scenario behaviours.
    pub uops: Vec<UOp>,
    /// First instruction of each usage-scenario behaviour (by [`ScenarioBehaviourId`]).
    pub ubeh: Vec<u32>,
    /// Number of external-call sites ([`SOp::Call`]).
    pub call_sites: u32,
}

impl Flat {
    /// `trace`: with the instructions that only write trace lines.
    pub fn build(beh: &[CBeh], act: &[CAct], ubeh: &[CUBeh], uact: &[CUAct], trace: bool) -> Flat {
        let mut c = Flat::default();
        for b in beh {
            c.sbeh.push(c.sops.len() as u32);
            let ops = &mut c.sops;
            let Some(start) = b.start else {
                ops.push(SOp::NoStart);
                continue;
            };
            if trace {
                ops.push(SOp::Start(start));
            }
            for &a in &b.chain {
                let series = match &act[a.index()] {
                    CAct::External { series, .. } => *series,
                    _ => None,
                };
                match series {
                    Some(se) => ops.push(SOp::BeginRt(a, se)),
                    None if trace => ops.push(SOp::Begin(a)),
                    None => {}
                }
                match &act[a.index()] {
                    CAct::Start | CAct::Stop => {}
                    CAct::Internal {
                        demands,
                        infra,
                        rescalls,
                    } => {
                        for &(prog, rt) in demands {
                            ops.push(SOp::Demand {
                                a,
                                prog,
                                rtype: rt.map_or(NO_TYPE, |t: ResourceTypeId| t.0),
                            });
                        }
                        for i in 0..infra.len() {
                            ops.push(SOp::Infra(a, i as u32));
                        }
                        for i in 0..rescalls.len() {
                            ops.push(SOp::ResCall(a, i as u32));
                        }
                    }
                    CAct::External { returns, .. } => {
                        ops.push(SOp::Call {
                            a,
                            site: c.call_sites,
                        });
                        c.call_sites += 1;
                        ops.push(SOp::Return {
                            a,
                            returns: !returns.is_empty(),
                        });
                    }
                    _ => ops.push(SOp::Exec(a)),
                }
                match series {
                    Some(se) => ops.push(SOp::EndRt(a, se)),
                    None if trace => ops.push(SOp::End(a)),
                    None => {}
                }
            }
            ops.push(if b.ends_at_stop {
                SOp::Ret
            } else {
                SOp::NoStop
            });
        }
        for b in ubeh {
            c.ubeh.push(c.uops.len() as u32);
            let ops = &mut c.uops;
            let Some(start) = b.start else {
                ops.push(UOp::NoStart);
                continue;
            };
            if trace {
                ops.push(UOp::Start(start));
            }
            for &a in &b.chain {
                let ua = &uact[a.index()];
                let series = match ua {
                    CUAct::Elsc { series, .. } => *series,
                    _ => None,
                };
                match series {
                    Some(_) => ops.push(UOp::BeginRt(a)),
                    None if trace => ops.push(UOp::Begin(a)),
                    None => {}
                }
                match ua {
                    CUAct::Start | CUAct::Stop => {}
                    CUAct::Delay(prog) => ops.push(UOp::Delay(a, *prog)),
                    CUAct::Elsc { .. } => {
                        ops.push(UOp::Call(a));
                        ops.push(UOp::Return(a));
                    }
                    CUAct::Branch { .. } | CUAct::Loop { .. } => ops.push(UOp::Exec(a)),
                }
                match series {
                    Some(se) => ops.push(UOp::EndRt(a, se)),
                    None if trace => ops.push(UOp::End(a)),
                    None => {}
                }
            }
            ops.push(if b.ends_at_stop {
                UOp::Ret
            } else {
                UOp::NoStop
            });
        }
        c
    }

    /// First instruction of SEFF behaviour `b`.
    #[inline]
    pub fn sbeh_start(&self, b: BehaviourId) -> u32 {
        self.sbeh[b.index()]
    }

    /// First instruction of usage-scenario behaviour `b`.
    #[inline]
    pub fn ubeh_start(&self, b: ScenarioBehaviourId) -> u32 {
        self.ubeh[b.index()]
    }
}

const _: () = assert!(std::mem::size_of::<SOp>() <= 16);
const _: () = assert!(std::mem::size_of::<UOp>() <= 12);
