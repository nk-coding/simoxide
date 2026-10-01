//! Conversion of model demands into service times, as done by the SimuCom framework wrappers
//! (`AbstractScheduledResource.consumeResource` with `ScheduledResource.calculateDemand` or
//! `SimulatedLinkingResource.calculateDemand`) before the scheduler sees them.
//!
//! The StoEx expressions involved (processing rate, throughput, latency) are evaluated by the
//! caller *on every call* in the order documented on each function (the reference re-evaluates
//! them per demand, so stochastic rates draw random numbers per demand).
//!
//! If the resulting demand is `<= 0` the reference does nothing at all: no demand measurement,
//! no scheduler call, the thread simply continues. Both functions return `None` in that case.
//! Otherwise the caller fires the resource-demand measurement (`fireDemand(concreteDemand)`) and
//! then calls the resource's `process`.

/// Processing resource: `demand / processingRate`.
///
/// Evaluation order in the reference: the demand StoEx (by the interpreter), then the processing
/// rate StoEx (`Context.evaluateStatic(processingRate, Double.class)`).
#[inline]
pub fn processing_demand(demand: f64, processing_rate: f64) -> Option<f64> {
    let concrete = demand / processing_rate;
    // `if (concreteDemand <= 0) return;` (NaN passes through, as in Java)
    if concrete <= 0.0 {
        None
    } else {
        Some(concrete)
    }
}

/// Error of [`linking_demand`]: `ThroughputZeroOrNegativeException`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThroughputNotPositive(pub f64);

impl std::fmt::Display for ThroughputNotPositive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "throughput of linking resource was less or equal zero ({})",
            self.0
        )
    }
}

impl std::error::Error for ThroughputNotPositive {}

/// Linking resource (FCFS): `(demand / throughput) / 1.0 + (0.0 + latency)`.
///
/// Evaluation order in the reference: throughput StoEx (`calculateDemand`, which throws if it is
/// `<= 0`), then the latency `DemandModifyingBehavior("1.0", latency)`: its scaling factor
/// `"1.0"` (scale = demand / 1.0), then the latency StoEx (additive value).
#[inline]
pub fn linking_demand(
    demand: f64,
    throughput: f64,
    latency: f64,
) -> Result<Option<f64>, ThroughputNotPositive> {
    if throughput <= 0.0 {
        return Err(ThroughputNotPositive(throughput));
    }
    let mut concrete = demand / throughput;
    let mut additive = 0.0;
    concrete /= 1.0;
    additive += latency;
    concrete += additive;
    Ok(if concrete <= 0.0 {
        None
    } else {
        Some(concrete)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processing() {
        assert_eq!(processing_demand(2.0, 4.0), Some(0.5));
        assert_eq!(processing_demand(0.0, 4.0), None);
        assert_eq!(processing_demand(-1.0, 4.0), None);
        assert!(processing_demand(f64::NAN, 1.0).unwrap().is_nan());
    }

    #[test]
    fn linking() {
        assert_eq!(linking_demand(100.0, 50.0, 0.25), Ok(Some(2.25)));
        assert_eq!(linking_demand(0.0, 50.0, 0.0), Ok(None));
        assert_eq!(linking_demand(0.0, 50.0, 0.1), Ok(Some(0.1)));
        assert!(linking_demand(1.0, 0.0, 0.0).is_err());
    }
}
