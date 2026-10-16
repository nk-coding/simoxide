//! Palladio's blackbox libraries, for hosts that run SimuLizar's reconfiguration rules or the
//! SPD transformations (SimOxide later): their signatures, and the registration of the
//! simulation time.
//!
//! - `org.palladiosimulator.simulizar.action.repository.black.SimulationStateLibrary`
//!   (`getSimulationTime() : Real`): [`register_simulation_time`] binds it to a host closure.
//! - `org.palladiosimulator.simulizar.action.repository.black.ProfilesLibrary` and
//!   `org.palladiosimulator.architecturaltemplates.catalog.black.ProfilesLibrary` (stereotype
//!   applications and tagged values of EMF Profiles): the host implements the operations it
//!   needs with `Natives::java`/`Natives::op` under the keys of [`catalog`]'s units
//!   (`<class>#<method>(<Java parameter types>)`, e.g.
//!   `org.palladiosimulator.simulizar.action.repository.black.ProfilesLibrary#hasAppliedStereotype(org.eclipse.emf.ecore.EObject,java.lang.String)`).
//!
//! The units are registered like Eclipse's `BlackboxRegistry`: put [`catalog`]'s units into a
//! [`super::BlackboxResolver`] (`registered` or `classpath`).

use super::{Catalog, Natives};
use qvto_model::ModelStore;
use qvto_runtime::Value;
use qvto_vm::ObjHandle;

/// The signatures (environments `simulizar` and `architecturaltemplates`).
pub const SIGNATURES: &str = include_str!("../../data/palladio-blackboxes.tsv");

/// The implementation key of `SimulationStateLibrary.getSimulationTime()`.
pub const GET_SIMULATION_TIME: &str =
    "org.palladiosimulator.simulizar.action.repository.black.SimulationStateLibrary#getSimulationTime()";

/// The units of Palladio's libraries.
pub fn catalog() -> Catalog {
    Catalog::parse(SIGNATURES).expect("palladio-blackboxes.tsv")
}

/// Binds `getSimulationTime()` to the host's simulation clock.
pub fn register_simulation_time<S: ModelStore + 'static>(
    n: &mut Natives<S>,
    time: impl Fn() -> f64 + 'static,
) where
    S::Obj: ObjHandle,
{
    n.op(GET_SIMULATION_TIME, move |_vm, _c| Ok(Value::Real(time())));
}
