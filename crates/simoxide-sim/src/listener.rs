//! Interpreter listeners (`docs/spec/simulation.md` SIM-14): a host observes
//! the interpretation as SimuLizar's `IInterpreterListener` lets an extension observe it
//! (`org.palladiosimulator.simulizar.interpreter.listener.IInterpreterListener`: begin/end of
//! usage scenarios, entry-level system calls, system operations, assembly-provided operations
//! and external calls, each with the simulated thread whose `RequestContext` links to its
//! parent), plus the creation and end of processes, which SimuLizar's events carry through the
//! thread.
//!
//! The engine calls the [`Listener`] registered with `Simulation::set_listener` /
//! `SsxRun::set_listener` (or [`crate::run_with_listener`]) with typed ids of the run's model and
//! the simulation time; it formats nothing and keeps no measurement state. A listener only
//! observes: it cannot change the run, so a run with one is the run without (same events, draws,
//! measurements, trace). Without a listener every call site is one predictable branch.
//!
//! Engines (the semantics per callback are on the [`Listener`] methods):
//!
//! * **Process engine, SimuLizar flavour** (exact and fast mode): every callback. Processes are
//!   the engine's processes (open-workload generators, users, forked behaviours).
//! * **Process engine, Slingshot flavour** (fast mode): the same. With quirk Q23 (SS-ACT-10,
//!   Slingshot's own behaviour) asynchronous forked behaviours are not run and the synchronous
//!   one runs in the forking process: such a fork creates no process.
//! * **Slingshot-exact engine** (`ssx`): Slingshot has no processes; a *process* is a Slingshot
//!   `User` (its id) or a synchronous forked behaviour of a `ForkAction` (its child
//!   interpretation context); asynchronous forked behaviours are not interpreted (Q23).
//!   Open-workload arrivals have no generator process. The callbacks follow Slingshot's bus
//!   events (SS-X-2): `UserStarted`/`UserFinished` for the usage scenario,
//!   `UserEntryRequested`/`UserRequestFinished` for the entry-level call and its system
//!   operation, `SEFFExternalActionCalled` and the caller's continuation for an external call,
//!   the provider's SEFF interpretation for its assembly operation.

use simoxide_model::{
    ActionId, AssemblyContextId, BehaviourId, ContainerId, RoleId, SignatureId, UserActionId,
};

/// A process of the run (the engine's process id: unique within the run, never reused).
pub type Pid = u64;

/// What a process runs ([`Listener::process_created`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessKind {
    /// The process that starts the users of an open workload (scenario index of
    /// `CompiledModel::scenarios`).
    OpenWorkload { scenario: u32 },
    /// A user of an open workload (one usage-scenario run).
    OpenUser { scenario: u32 },
    /// A user of a closed workload (its iterations are its usage-scenario runs).
    ClosedUser { scenario: u32 },
    /// A forked behaviour `behaviour` of `ForkAction` `fork`, synchronous (joined at the fork's
    /// synchronisation point) or asynchronous.
    Forked {
        fork: ActionId,
        behaviour: BehaviourId,
        synchronous: bool,
    },
}

/// An assembly-provided operation ([`Listener::assembly_operation`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssemblyOp {
    /// The assembly context whose component provides the operation.
    pub assembly: AssemblyContextId,
    pub role: RoleId,
    pub signature: SignatureId,
    /// The container the assembly context is allocated to when the operation starts (`None`:
    /// not allocated; a container created by a model change, e.g. a scaled-out pod, has an id
    /// of its own).
    pub container: Option<ContainerId>,
}

/// Observer of the interpretation. Every method has an empty default; `t` is the simulation
/// time in model time units (seconds of the run's clock), `p` the process the step belongs to.
pub trait Listener {
    /// Process `p` was created by `parent` (the process that ran when it was created; `None`:
    /// none, e.g. the initial generators and closed users, Slingshot-exact users).
    fn process_created(&mut self, t: f64, p: Pid, parent: Option<Pid>, kind: ProcessKind) {
        let _ = (t, p, parent, kind);
    }
    /// Process `p` ended (also when it was aborted).
    fn process_ended(&mut self, t: f64, p: Pid) {
        let _ = (t, p);
    }
    /// A usage-scenario run (`scenario`: index of `CompiledModel::scenarios`) begins or ends in
    /// user process `p` (MEAS-13's session).
    fn usage_scenario(&mut self, t: f64, begin: bool, p: Pid, scenario: u32) {
        let _ = (t, begin, p, scenario);
    }
    /// An entry-level system call begins or ends.
    fn entry_call(&mut self, t: f64, begin: bool, p: Pid, call: UserActionId) {
        let _ = (t, begin, p, call);
    }
    /// A system operation (the system's provided role and signature an entry-level call calls)
    /// begins or ends.
    fn system_operation(&mut self, t: f64, begin: bool, p: Pid, role: RoleId, sig: SignatureId) {
        let _ = (t, begin, p, role, sig);
    }
    /// An assembly-provided operation begins (the provider's SEFF, or a composite's
    /// delegation, is entered) or ends. Begin and end nest per process.
    fn assembly_operation(&mut self, t: f64, begin: bool, p: Pid, op: &AssemblyOp) {
        let _ = (t, begin, p, op);
    }
    /// An `ExternalCallAction` begins (the call starts) or ends (the caller continues).
    fn external_call(&mut self, t: f64, begin: bool, p: Pid, action: ActionId) {
        let _ = (t, begin, p, action);
    }
}
