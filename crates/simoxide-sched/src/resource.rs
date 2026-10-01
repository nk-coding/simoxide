//! Policy selection and a uniform wrapper over the active resource models.

use crate::common::{Completion, ResourceListener, SchedError, Wakeup};
use crate::delay::Delay;
use crate::fcfs::Fcfs;
use crate::ps::ProcessorSharing;
use crate::ps_vt::VirtualTimeProcessorSharing;
use crate::time::SimTime;

/// Scheduling policy of a processing resource, from the PCM `SchedulingPolicy` id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchedulingPolicy {
    /// `ProcessorSharing` -> `SimProcessorSharingResource` with `numberOfReplicas` cores.
    ProcessorSharing,
    /// `FCFS` -> `SimFCFSResource` (always one server, whatever `numberOfReplicas` says).
    Fcfs,
    /// `Delay` -> `SimDelayResource`.
    Delay,
}

impl SchedulingPolicy {
    /// Maps the id of a PCM `SchedulingPolicy` (`processingResource.getSchedulingPolicy().getId()`)
    /// like `AbstractScheduledResource`'s constructor and `ScheduledResource.getScheduledResource`.
    ///
    /// Any other id is looked up as a scheduler *extension* in the reference (exact schedulers,
    /// `SPECIAL_WINDOWS`, `SPECIAL_LINUXO1`, ...), which is out of scope: `None`.
    pub fn from_pcm_id(id: &str) -> Option<Self> {
        match id {
            "ProcessorSharing" | "PROCESSOR_SHARING" => Some(Self::ProcessorSharing),
            "FCFS" => Some(Self::Fcfs),
            "Delay" | "DELAY" => Some(Self::Delay),
            _ => None,
        }
    }
}

/// Which processor-sharing implementation to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PsAlgorithm {
    /// Bit-exact port of the reference (O(n) per event). Default.
    #[default]
    Exact,
    /// O(log n) per event, not bit-exact (see [`VirtualTimeProcessorSharing`]).
    VirtualTime,
}

/// Any active resource model, dispatching statically on the variant.
#[derive(Clone, Debug)]
pub enum ActiveResource<J> {
    /// Exact processor sharing.
    Ps(ProcessorSharing<J>),
    /// Virtual-time processor sharing.
    PsVirtualTime(VirtualTimeProcessorSharing<J>),
    /// First-come-first-served.
    Fcfs(Fcfs<J>),
    /// Delay.
    Delay(Delay),
}

impl<J: Copy + PartialEq> ActiveResource<J> {
    /// Creates the resource SimuLizar creates for a processing resource with `policy` and
    /// `replicas` (`numberOfReplicas`). Linking resources are always `Fcfs`.
    pub fn new(policy: SchedulingPolicy, replicas: u32, ps: PsAlgorithm) -> Self {
        match (policy, ps) {
            (SchedulingPolicy::ProcessorSharing, PsAlgorithm::Exact) => {
                Self::Ps(ProcessorSharing::new(replicas))
            }
            (SchedulingPolicy::ProcessorSharing, PsAlgorithm::VirtualTime) => {
                Self::PsVirtualTime(VirtualTimeProcessorSharing::new(replicas))
            }
            (SchedulingPolicy::Fcfs, _) => Self::Fcfs(Fcfs::new()),
            (SchedulingPolicy::Delay, _) => Self::Delay(Delay::new()),
        }
    }

    /// Number of scheduler cores (1 for FCFS and delay).
    pub fn cores(&self) -> u32 {
        match self {
            Self::Ps(r) => r.cores(),
            Self::PsVirtualTime(r) => r.cores(),
            Self::Fcfs(_) | Self::Delay(_) => 1,
        }
    }

    /// `getQueueLengthFor(_, core)` (FCFS and delay ignore `core`).
    pub fn queue_length(&self, core: u32) -> u64 {
        match self {
            Self::Ps(r) => r.queue_length(core),
            Self::PsVirtualTime(r) => r.queue_length(core),
            Self::Fcfs(r) => r.len() as u64,
            Self::Delay(r) => r.len() as u64,
        }
    }

    /// `TakeScheduledResourceUtilization`: fraction of the `instances` replicas (the
    /// `numberOfReplicas` of the model, which for FCFS/delay may differ from [`Self::cores`])
    /// whose queue length is positive.
    pub fn busy_fraction(&self, instances: u32) -> f64 {
        let busy = (0..instances).filter(|&c| self.queue_length(c) > 0).count();
        busy as f64 / instances as f64
    }

    /// `IActiveResource.process(...)` with the concrete demand (seconds of service).
    // always: keeps it inlined when several simulator monomorphizations call it
    #[inline(always)]
    pub fn process<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        job: J,
        demand: f64,
        listener: &mut L,
    ) -> Result<Wakeup<J>, SchedError> {
        match self {
            Self::Ps(r) => Ok(r.process(now, job, demand, listener)),
            Self::PsVirtualTime(r) => Ok(r.process(now, job, demand, listener)),
            Self::Fcfs(r) => r.process(now, job, demand, listener),
            Self::Delay(r) => Ok(r.process(now, job, demand, listener)),
        }
    }

    /// Delivers a wake-up; `None` if it is stale. An error aborts the run (see [`Fcfs`]).
    #[inline(always)]
    pub fn on_wakeup<L: ResourceListener<J>>(
        &mut self,
        now: SimTime,
        wakeup: &Wakeup<J>,
        listener: &mut L,
    ) -> Result<Option<Completion<J>>, SchedError> {
        match self {
            Self::Ps(r) => Ok(r.on_wakeup(now, wakeup, listener)),
            Self::PsVirtualTime(r) => Ok(r.on_wakeup(now, wakeup, listener)),
            Self::Fcfs(r) => r.on_wakeup(now, wakeup, listener),
            Self::Delay(r) => Ok(r.on_wakeup(now, wakeup, listener)),
        }
    }

    /// `true` if `wakeup` is the resource's currently scheduled event. A superseded (stale)
    /// wake-up corresponds to an event the reference removed from its event list
    /// (`removeEvent()`), so an event core must not treat it as a processed event (e.g. for
    /// stop conditions checked after every event).
    pub fn is_current(&self, wakeup: &Wakeup<J>) -> bool {
        let pending = match self {
            Self::Ps(r) => r.pending(),
            Self::PsVirtualTime(r) => r.pending(),
            Self::Fcfs(r) => r.pending(),
            Self::Delay(r) => return r.is_current(wakeup),
        };
        pending.is_some_and(|p| p.generation() == wakeup.generation())
    }

    /// The currently scheduled wake-up of a PS or FCFS resource (`None` if idle, and always
    /// `None` for [`Delay`], whose wake-ups are per job).
    #[inline]
    pub fn pending_wakeup(&self) -> Option<Wakeup<J>> {
        match self {
            Self::Ps(r) => r.pending(),
            Self::PsVirtualTime(r) => r.pending(),
            Self::Fcfs(r) => r.pending(),
            Self::Delay(_) => None,
        }
    }

    /// `IActiveResource.stop()`.
    pub fn stop(&mut self) {
        match self {
            Self::Ps(r) => r.stop(),
            Self::PsVirtualTime(r) => r.stop(),
            Self::Fcfs(r) => r.stop(),
            Self::Delay(r) => r.stop(),
        }
    }
}
