//! The HPA controller's arithmetic (`docs/spec/hpa.md`, HPA-2 to HPA-5): a port of Kubernetes'
//! `pkg/controller/podautoscaler` (`horizontal.go`: `reconcileAutoscaler`,
//! `stabilizeRecommendation[WithBehaviors]`, `convertDesiredReplicasWith{Rules,BehaviorRate}`,
//! `calculateScale{Up,Down}Limit…`, `storeScaleEvent`; `replica_calculator.go`:
//! `GetResourceReplicas`, `groupPods`; `metrics/utilization.go`: `GetResourceUtilizationRatio`).
//!
//! Plain data and integer arithmetic where Kubernetes uses integers (replica counts, milli-cores,
//! the utilisation percentage); times are simulation nanoseconds. Nothing here knows the
//! simulator: [`reconcile`] takes the current replica count and the pods' samples and returns the
//! [`Decision`]; [`Replicas`] keeps the pods that are starting (not ready yet).

/// A `HPAScalingPolicy`: `Pods` or `Percent`, per `periodSeconds`.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScalingPolicy {
    pub kind: PolicyKind,
    pub value: i32,
    /// `periodSeconds`, in ns.
    pub period_ns: i64,
}

#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyKind {
    Pods,
    Percent,
}

/// `selectPolicy`.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    Max,
    Min,
    Disabled,
}

/// `HPAScalingRules` of one direction, defaulted (`GenerateHPAScale{Up,Down}Rules`).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// `stabilizationWindowSeconds`, in ns.
    pub stabilization_ns: i64,
    pub select: Select,
    pub policies: Vec<ScalingPolicy>,
    /// `tolerance` (the `HPAConfigurableTolerance` field); `None`: the controller's.
    pub tolerance: Option<f64>,
}

impl Rules {
    /// Kubernetes' default scale-up rules: no stabilisation, the larger of 4 pods and 100 % per
    /// 15 s (`defaultHPAScaleUpRules`).
    pub fn default_up() -> Rules {
        Rules {
            stabilization_ns: 0,
            select: Select::Max,
            policies: vec![
                ScalingPolicy {
                    kind: PolicyKind::Pods,
                    value: 4,
                    period_ns: 15 * SEC,
                },
                ScalingPolicy {
                    kind: PolicyKind::Percent,
                    value: 100,
                    period_ns: 15 * SEC,
                },
            ],
            tolerance: None,
        }
    }

    /// Kubernetes' default scale-down rules: 100 % per 15 s; the stabilisation window is the
    /// controller's (`--horizontal-pod-autoscaler-downscale-stabilization`, given here).
    pub fn default_down(stabilization_ns: i64) -> Rules {
        Rules {
            stabilization_ns,
            select: Select::Max,
            policies: vec![ScalingPolicy {
                kind: PolicyKind::Percent,
                value: 100,
                period_ns: 15 * SEC,
            }],
            tolerance: None,
        }
    }

    fn longest_period(&self) -> i64 {
        self.policies.iter().map(|p| p.period_ns).max().unwrap_or(0)
    }
}

/// `spec.behavior` (defaulted).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct Behavior {
    pub up: Rules,
    pub down: Rules,
}

/// One second in ns.
pub const SEC: i64 = 1_000_000_000;

/// What the controller needs of one HPA: its spec (CPU utilisation target) and the controller
/// settings that apply to it.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub min_replicas: i32,
    pub max_replicas: i32,
    /// `target.averageUtilization` (percent of the request).
    pub target_utilization: i32,
    /// `None`: no `spec.behavior`, the controller's legacy path (`normalizeDesiredReplicas`).
    pub behavior: Option<Behavior>,
    /// `--horizontal-pod-autoscaler-tolerance` (default 0.1).
    pub tolerance: f64,
    /// `--horizontal-pod-autoscaler-downscale-stabilization` (default 300 s), in ns.
    pub downscale_stabilization_ns: i64,
}

/// The tolerances of the two directions (`Tolerances`, `tolerancesForHpa`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerances {
    pub down: f64,
    pub up: f64,
}

impl Tolerances {
    /// `isWithin`: `1 - down <= ratio <= 1 + up`.
    pub fn within(self, ratio: f64) -> bool {
        1.0 - self.down <= ratio && ratio <= 1.0 + self.up
    }
}

impl Spec {
    pub fn tolerances(&self) -> Tolerances {
        let t = self.tolerance;
        match &self.behavior {
            Some(b) => Tolerances {
                down: b.down.tolerance.unwrap_or(t),
                up: b.up.tolerance.unwrap_or(t),
            },
            None => Tolerances { down: t, up: t },
        }
    }
}

/// A timestamped recommendation (`timestampedRecommendation`).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recommendation {
    pub replicas: i32,
    pub at_ns: i64,
}

/// A scale event (`timestampedScaleEvent`).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleEvent {
    pub change: i32,
    pub at_ns: i64,
    pub outdated: bool,
}

/// The controller's memory of one HPA (`recommendations`, `scaleUpEvents`, `scaleDownEvents`).
/// Bounded: old entries are replaced, not appended (at most one entry per sync inside the
/// longest window or period, plus one).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HpaMemory {
    pub recommendations: Vec<Recommendation>,
    pub up_events: Vec<ScaleEvent>,
    pub down_events: Vec<ScaleEvent>,
}

/// A pod as the replica calculator sees it (`groupPods`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pod {
    /// The CPU request (milli-cores).
    pub request_milli: i64,
    pub state: PodState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodState {
    /// Ready, with its usage (milli-cores) over the metric window.
    Ready(i64),
    /// Unready (pending, or ready for less than one metric window inside the CPU
    /// initialisation period): set aside, 0 % when scaling up.
    Unready,
    /// No metric: 100 % of the request (or the target if higher) when scaling down, 0 % up.
    Missing,
}

/// The replica calculator's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calc {
    pub replicas: i32,
    /// The ready pods' utilisation (percent of their requests) and its ratio to the target.
    pub utilization: i32,
    pub ratio: f64,
    pub ready: usize,
    pub unready: usize,
    pub missing: usize,
}

/// `GetResourceUtilizationRatio`: the integer percentage `Σ usage · 100 / Σ request` and its
/// ratio to the target.
pub fn utilization_ratio(metrics: &[(i64, i64)], target: i32) -> Result<(f64, i32), String> {
    let usage: i128 = metrics.iter().map(|m| i128::from(m.0)).sum();
    let request: i128 = metrics.iter().map(|m| i128::from(m.1)).sum();
    if request == 0 {
        return Err("no metrics returned matched known pods".into());
    }
    let pct = (usage * 100).div_euclid(request);
    let pct = pct.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32;
    Ok((f64::from(pct) / f64::from(target), pct))
}

fn ceil_i32(x: f64) -> i32 {
    x.ceil() as i32
}

/// `GetResourceReplicas` for a CPU utilisation target: the replica count the pods' samples ask
/// for (HPA-3).
pub fn resource_replicas(
    current: i32,
    pods: &[Pod],
    target: i32,
    tol: Tolerances,
) -> Result<Calc, String> {
    if pods.is_empty() {
        return Err("no pods returned by selector while calculating replica count".into());
    }
    // (usage, request) of the pods with a metric
    let mut metrics: Vec<(i64, i64)> = Vec::with_capacity(pods.len());
    let (mut unready, mut missing) = (0, 0);
    for p in pods {
        match p.state {
            PodState::Ready(u) => metrics.push((u, p.request_milli)),
            PodState::Unready => unready += 1,
            PodState::Missing => missing += 1,
        }
    }
    let ready = metrics.len();
    if metrics.is_empty() {
        return Err("did not receive metrics for targeted pods (pods might be unready)".into());
    }
    let (ratio, utilization) = utilization_ratio(&metrics, target)?;
    let calc = |replicas| Calc {
        replicas,
        utilization,
        ratio,
        ready,
        unready,
        missing,
    };
    let scale_up_with_unready = unready > 0 && ratio > 1.0;
    if !scale_up_with_unready && missing == 0 {
        if tol.within(ratio) {
            return Ok(calc(current));
        }
        return Ok(calc(ceil_i32(ratio * ready as f64)));
    }
    if missing > 0 {
        let fallback = i64::from(target.max(100));
        for p in pods {
            if p.state == PodState::Missing {
                if ratio < 1.0 {
                    metrics.push((p.request_milli * fallback / 100, p.request_milli));
                } else if ratio > 1.0 {
                    metrics.push((0, p.request_milli));
                }
            }
        }
    }
    if scale_up_with_unready {
        for p in pods {
            if p.state == PodState::Unready {
                metrics.push((0, p.request_milli));
            }
        }
    }
    let (new_ratio, _) = utilization_ratio(&metrics, target)?;
    if tol.within(new_ratio) || (ratio < 1.0 && new_ratio > 1.0) || (ratio > 1.0 && new_ratio < 1.0)
    {
        return Ok(calc(current));
    }
    let new_replicas = ceil_i32(new_ratio * metrics.len() as f64);
    if (new_ratio < 1.0 && new_replicas > current) || (new_ratio > 1.0 && new_replicas < current) {
        return Ok(calc(current));
    }
    Ok(calc(new_replicas))
}

/// The outcome of one reconciliation (HPA-2), for the decision log.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// `currentReplicas` (the scale's `spec.replicas`: ready plus starting pods).
    pub current: i32,
    /// The replica calculator's result (`None`: not computed: outside `[min, max]`, or an
    /// error).
    pub calc: Option<Calc>,
    /// After stabilisation (`None` if not computed).
    pub stabilized: Option<i32>,
    /// The new replica count.
    pub desired: i32,
    /// Why `desired` is what it is: `DesiredWithinRange`, `ScaleUpLimit`, `TooManyReplicas`,
    /// `TooFewReplicas`, `ScaleDownLimit`, `AboveMax`, `BelowMin`, `ScalingDisabled`,
    /// `FailedGetResourceMetric`.
    pub reason: &'static str,
    /// The calculator's error (no rescale).
    pub error: Option<String>,
}

impl Decision {
    pub fn rescale(&self) -> bool {
        self.desired != self.current
    }
}

/// One sync of an HPA (`reconcileAutoscaler`, HPA-2): `current` replicas, the pods' samples,
/// at `now`. Records the recommendation and, if it rescales, the scale event.
pub fn reconcile(
    spec: &Spec,
    mem: &mut HpaMemory,
    now: i64,
    current: i32,
    pods: &[Pod],
) -> Decision {
    // recordInitialRecommendation
    if mem.recommendations.is_empty() {
        mem.recommendations.push(Recommendation {
            replicas: current,
            at_ns: now,
        });
    }
    let min = spec.min_replicas;
    let max = spec.max_replicas;
    let mut d = Decision {
        current,
        calc: None,
        stabilized: None,
        desired: current,
        reason: "DesiredWithinRange",
        error: None,
    };
    if current == 0 && min != 0 {
        d.desired = 0;
        d.reason = "ScalingDisabled";
        return d;
    } else if current > max {
        d.desired = max;
        d.reason = "AboveMax";
    } else if current < min {
        d.desired = min;
        d.reason = "BelowMin";
    } else {
        let calc =
            match resource_replicas(current, pods, spec.target_utilization, spec.tolerances()) {
                Ok(c) => c,
                Err(e) => {
                    d.reason = "FailedGetResourceMetric";
                    d.error = Some(e);
                    return d;
                }
            };
        d.calc = Some(calc);
        let metric_desired = calc.replicas;
        let (stabilized, desired, reason) = match &spec.behavior {
            None => {
                let s = stabilize_legacy(spec, mem, now, metric_desired);
                let (r, why) = with_rules(current, s, min, max);
                (s, r, why)
            }
            Some(b) => {
                let s = stabilize_with_behaviors(b, mem, now, current, metric_desired);
                let (r, why) = with_behavior_rate(b, mem, now, current, s, min, max);
                (s, r, why)
            }
        };
        d.stabilized = Some(stabilized);
        d.desired = desired;
        d.reason = reason;
    }
    if d.rescale() {
        store_scale_event(spec, mem, now, current, d.desired);
    }
    d
}

/// `stabilizeRecommendation` (no `behavior`): the maximum of the recommendations of the
/// downscale stabilisation window and this one; records this one.
fn stabilize_legacy(spec: &Spec, mem: &mut HpaMemory, now: i64, desired: i32) -> i32 {
    let mut max = desired;
    let cutoff = now - spec.downscale_stabilization_ns;
    let mut old = None;
    for (i, r) in mem.recommendations.iter().enumerate() {
        if r.at_ns < cutoff {
            old = Some(i);
        } else if r.replicas > max {
            max = r.replicas;
        }
    }
    record(mem, old, desired, now);
    max
}

fn record(mem: &mut HpaMemory, old: Option<usize>, replicas: i32, now: i64) {
    let r = Recommendation {
        replicas,
        at_ns: now,
    };
    match old {
        Some(i) => mem.recommendations[i] = r,
        None => mem.recommendations.push(r),
    }
}

/// `stabilizeRecommendationWithBehaviors`.
fn stabilize_with_behaviors(
    b: &Behavior,
    mem: &mut HpaMemory,
    now: i64,
    current: i32,
    desired: i32,
) -> i32 {
    let up_cutoff = now - b.up.stabilization_ns;
    let down_cutoff = now - b.down.stabilization_ns;
    let (mut up, mut down) = (desired, desired);
    let mut old = None;
    for (i, r) in mem.recommendations.iter().enumerate() {
        if r.at_ns > up_cutoff {
            up = up.min(r.replicas);
        }
        if r.at_ns > down_cutoff {
            down = down.max(r.replicas);
        }
        if r.at_ns < up_cutoff && r.at_ns < down_cutoff {
            old = Some(i);
        }
    }
    let mut rec = current;
    if rec < up {
        rec = up;
    }
    if rec > down {
        rec = down;
    }
    record(mem, old, desired, now);
    rec
}

/// `convertDesiredReplicasWithRules` (no `behavior`): `[min, min(max, max(2·current, 4))]`.
fn with_rules(current: i32, desired: i32, min: i32, max: i32) -> (i32, &'static str) {
    let scale_up_limit = (2.0 * f64::from(current)).max(4.0) as i32;
    let (maximum, why) = if max > scale_up_limit {
        (scale_up_limit, "ScaleUpLimit")
    } else {
        (max, "TooManyReplicas")
    };
    if desired < min {
        (min, "TooFewReplicas")
    } else if desired > maximum {
        (maximum, why)
    } else {
        (desired, "DesiredWithinRange")
    }
}

/// `getReplicasChangePerPeriod`.
fn change_in_period(period: i64, events: &[ScaleEvent], now: i64) -> i32 {
    let cutoff = now - period;
    events
        .iter()
        .filter(|e| e.at_ns > cutoff)
        .map(|e| e.change)
        .sum()
}

/// `calculateScaleUpLimitWithScalingRules`.
pub fn scale_up_limit(current: i32, mem: &HpaMemory, rules: &Rules, now: i64) -> i32 {
    let (mut result, pick): (i32, fn(i32, i32) -> i32) = match rules.select {
        Select::Disabled => return current,
        Select::Min => (i32::MAX, i32::min),
        Select::Max => (i32::MIN, i32::max),
    };
    for p in &rules.policies {
        let added = change_in_period(p.period_ns, &mem.up_events, now);
        let deleted = change_in_period(p.period_ns, &mem.down_events, now);
        let start = current - added + deleted;
        let proposed = match p.kind {
            PolicyKind::Pods => start + p.value,
            PolicyKind::Percent => ceil_i32(f64::from(start) * (1.0 + f64::from(p.value) / 100.0)),
        };
        result = pick(result, proposed);
    }
    result
}

/// `calculateScaleDownLimitWithBehaviors`.
pub fn scale_down_limit(current: i32, mem: &HpaMemory, rules: &Rules, now: i64) -> i32 {
    let (mut result, pick): (i32, fn(i32, i32) -> i32) = match rules.select {
        Select::Disabled => return current,
        Select::Min => (i32::MIN, i32::max),
        Select::Max => (i32::MAX, i32::min),
    };
    for p in &rules.policies {
        let added = change_in_period(p.period_ns, &mem.up_events, now);
        let deleted = change_in_period(p.period_ns, &mem.down_events, now);
        let start = current - added + deleted;
        let proposed = match p.kind {
            PolicyKind::Pods => start - p.value,
            PolicyKind::Percent => (f64::from(start) * (1.0 - f64::from(p.value) / 100.0)) as i32,
        };
        result = pick(result, proposed);
    }
    result
}

/// `convertDesiredReplicasWithBehaviorRate`.
fn with_behavior_rate(
    b: &Behavior,
    mem: &HpaMemory,
    now: i64,
    current: i32,
    desired: i32,
    min: i32,
    max: i32,
) -> (i32, &'static str) {
    if desired > current {
        let limit = scale_up_limit(current, mem, &b.up, now).max(current);
        let (maximum, why) = if max > limit {
            (limit, "ScaleUpLimit")
        } else {
            (max, "TooManyReplicas")
        };
        if desired > maximum {
            return (maximum, why);
        }
    } else if desired < current {
        let limit = scale_down_limit(current, mem, &b.down, now).min(current);
        let (minimum, why) = if min < limit {
            (limit, "ScaleDownLimit")
        } else {
            (min, "TooFewReplicas")
        };
        if desired < minimum {
            return (minimum, why);
        }
    }
    (desired, "DesiredWithinRange")
}

/// `storeScaleEvent` (only with `behavior`): the change, replacing an event older than the
/// direction's longest policy period.
fn store_scale_event(spec: &Spec, mem: &mut HpaMemory, now: i64, prev: i32, new: i32) {
    let Some(b) = &spec.behavior else {
        return;
    };
    let (events, rules, change) = if new > prev {
        (&mut mem.up_events, &b.up, new - prev)
    } else {
        (&mut mem.down_events, &b.down, prev - new)
    };
    let longest = rules.longest_period();
    for e in events.iter_mut() {
        if e.at_ns + longest < now {
            e.outdated = true;
        }
    }
    let ev = ScaleEvent {
        change,
        at_ns: now,
        outdated: false,
    };
    match events.iter_mut().find(|e| e.outdated) {
        Some(e) => *e = ev,
        None => events.push(ev),
    }
}

// ---------------------------------------------------------------------------------------------

/// The pods of one HPA's target that are starting (created, not ready yet: HPA-6): their ready
/// times, in creation order. The ready pods are the target group's elements.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replicas {
    pub pending: Vec<i64>,
}

/// What a rescale does to the target (HPA-6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// New pods that start now (ready after the start-up delay).
    pub started: i32,
    /// Starting pods deleted (the newest first).
    pub cancelled: i32,
    /// The ready pods' new count, if it changes now (an absolute adjustment of the group).
    pub goal: Option<i32>,
}

impl Replicas {
    /// `desired` replicas from `ready` ready pods at `now`: a scale-up starts pods (ready at
    /// `now + startup`, at once if `startup` is 0); a scale-down deletes starting pods first
    /// (the ReplicaSet controller deletes pods that are not ready before ready ones, newer
    /// before older), then ready pods.
    pub fn rescale(&mut self, ready: i32, desired: i32, now: i64, startup: i64) -> Plan {
        let current = ready + self.pending.len() as i32;
        let mut plan = Plan {
            started: 0,
            cancelled: 0,
            goal: None,
        };
        if desired > current {
            plan.started = desired - current;
            if startup <= 0 {
                plan.goal = Some(ready + plan.started);
            } else {
                self.pending
                    .extend(std::iter::repeat_n(now + startup, plan.started as usize));
            }
        } else if desired < current {
            let mut remove = current - desired;
            while remove > 0 && !self.pending.is_empty() {
                self.pending.pop();
                plan.cancelled += 1;
                remove -= 1;
            }
            if remove > 0 {
                plan.goal = Some(ready - remove);
            }
        }
        plan
    }

    /// The starting pods ready at `now` leave the list: their number.
    pub fn due(&mut self, now: i64) -> i32 {
        let n = self.pending.iter().take_while(|&&t| t <= now).count();
        self.pending.drain(..n);
        n as i32
    }

    /// The earliest ready time.
    pub fn next(&self) -> Option<i64> {
        self.pending.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = SEC;

    fn spec(behavior: Option<Behavior>) -> Spec {
        Spec {
            min_replicas: 1,
            max_replicas: 10,
            target_utilization: 50,
            behavior,
            tolerance: 0.1,
            downscale_stabilization_ns: 300 * S,
        }
    }

    fn default_behavior() -> Behavior {
        Behavior {
            up: Rules::default_up(),
            down: Rules::default_down(300 * S),
        }
    }

    fn ready(usage: &[i64]) -> Vec<Pod> {
        usage
            .iter()
            .map(|&u| Pod {
                request_milli: 1000,
                state: PodState::Ready(u),
            })
            .collect()
    }

    fn tol() -> Tolerances {
        Tolerances { down: 0.1, up: 0.1 }
    }

    #[test]
    fn proportional_rule_and_tolerance() {
        // 2 pods at 90 % against 50 %: ratio 1.8, ceil(1.8 * 2) = 4
        let c = resource_replicas(2, &ready(&[900, 900]), 50, tol()).unwrap();
        assert_eq!((c.replicas, c.utilization), (4, 90));
        assert_eq!(c.ratio, 1.8);
        // 54 %: ratio 1.08 inside the tolerance: unchanged
        let c = resource_replicas(2, &ready(&[540, 540]), 50, tol()).unwrap();
        assert_eq!(c.replicas, 2);
        // exactly 1.1 is inside (inclusive band), 1.12 is not: ceil(1.12 * 2) = 3
        assert_eq!(
            resource_replicas(2, &ready(&[550, 550]), 50, tol())
                .unwrap()
                .replicas,
            2
        );
        assert_eq!(
            resource_replicas(2, &ready(&[560, 560]), 50, tol())
                .unwrap()
                .replicas,
            3
        );
        // 0.9 is inside, 0.88 is not: ceil(0.88 * 4) = 4 (3.52)
        assert_eq!(
            resource_replicas(4, &ready(&[450; 4]), 50, tol())
                .unwrap()
                .replicas,
            4
        );
        assert_eq!(
            resource_replicas(4, &ready(&[440; 4]), 50, tol())
                .unwrap()
                .replicas,
            4
        );
        // 10 %: ceil(0.2 * 4) = 1
        assert_eq!(
            resource_replicas(4, &ready(&[100; 4]), 50, tol())
                .unwrap()
                .replicas,
            1
        );
        // the integer percentage: 0.5996 cores of 1 is 59 %, not 59.96 %
        let c = resource_replicas(1, &ready(&[599]), 50, tol()).unwrap();
        assert_eq!((c.utilization, c.ratio), (59, 1.18));
        assert_eq!(c.replicas, 2);
    }

    #[test]
    fn unready_pods_dampen_a_scale_up() {
        // 2 ready at 100 % (ratio 2.0) and 2 unready: the unready count as 0 %:
        // new ratio (2000 / 4000 = 50 %) = 1.0: inside the tolerance -> current (4)
        let mut pods = ready(&[1000, 1000]);
        pods.extend(
            [Pod {
                request_milli: 1000,
                state: PodState::Unready,
            }; 2],
        );
        let c = resource_replicas(4, &pods, 50, tol()).unwrap();
        assert_eq!((c.replicas, c.ready, c.unready), (4, 2, 2));
        // 2 ready at 100 % and 1 unready, target 40 %: ratio 2.5; with the unready at 0 %:
        // 66 % / 40 = 1.65 -> ceil(1.65 * 3) = 5
        let mut pods = ready(&[1000, 1000]);
        pods.push(Pod {
            request_milli: 1000,
            state: PodState::Unready,
        });
        assert_eq!(resource_replicas(3, &pods, 40, tol()).unwrap().replicas, 5);
        // scaling down, the unready pods are left out: 2 ready at 10 % of 50 %:
        // ceil(0.2 * 2) = 1
        let mut pods = ready(&[100, 100]);
        pods.push(Pod {
            request_milli: 1000,
            state: PodState::Unready,
        });
        assert_eq!(resource_replicas(3, &pods, 50, tol()).unwrap().replicas, 1);
        // no ready pod: an error (no rescale)
        let pods = [Pod {
            request_milli: 1000,
            state: PodState::Unready,
        }];
        assert!(resource_replicas(1, &pods, 50, tol()).is_err());
    }

    #[test]
    fn missing_pods_are_conservative() {
        // scale-down: a missing pod counts as 100 % of its request:
        // ready 2 x 10 % -> ratio 0.2; with the missing at 100 %: 1200 / 3000 = 40 % -> 0.8
        // -> ceil(0.8 * 3) = 3 = current (no change)
        let mut pods = ready(&[100, 100]);
        pods.push(Pod {
            request_milli: 1000,
            state: PodState::Missing,
        });
        assert_eq!(resource_replicas(3, &pods, 50, tol()).unwrap().replicas, 3);
        // scale-up: 0 %: 2 x 100 % + 0 -> 66 % / 50 = 1.32 -> ceil(1.32 * 3) = 4 (3.96)
        let mut pods = ready(&[1000, 1000]);
        pods.push(Pod {
            request_milli: 1000,
            state: PodState::Missing,
        });
        assert_eq!(resource_replicas(3, &pods, 50, tol()).unwrap().replicas, 4);
    }

    #[test]
    fn legacy_path_limits_and_stabilises() {
        let s = spec(None);
        let mut m = HpaMemory::default();
        // 1 pod at 100 % of 50 %: 2; the scale-up limit max(2 * 1, 4) = 4 does not bite
        let d = reconcile(&s, &mut m, 15 * S, 1, &ready(&[1000]));
        assert_eq!((d.desired, d.reason), (2, "DesiredWithinRange"));
        // 2 pods at 500 %: 20 -> limited to max(2 * 2, 4) = 4
        let d = reconcile(&s, &mut m, 30 * S, 2, &ready(&[5000, 5000]));
        assert_eq!((d.desired, d.reason), (4, "ScaleUpLimit"));
        // idle: the recommendation 1 is stabilised by the window's maximum (20) -> max 10,
        // within the up limit 8: the stabilised 20 is clamped to the rules' maximum 8
        let d = reconcile(&s, &mut m, 45 * S, 4, &ready(&[0; 4]));
        assert_eq!(d.stabilized, Some(20));
        assert_eq!(d.desired, 8);
        // the legacy path also stabilises upwards
        assert_eq!(m.up_events.len(), 0, "no scale events without behavior");
    }

    #[test]
    fn scale_down_waits_for_the_stabilisation_window() {
        let s = spec(Some(default_behavior()));
        let mut m = HpaMemory::default();
        let busy = ready(&[1000; 4]);
        // t = 15: 4 pods at 100 % of 50 % -> 8
        let d = reconcile(&s, &mut m, 15 * S, 4, &busy);
        assert_eq!(d.desired, 8);
        // then idle from t = 30 on (recommendation 0): the stabilised value is the maximum of
        // the last 300 s, which holds the 16 and the initial 4 of t = 15 until t = 300
        let idle = ready(&[0; 8]);
        let mut t = 30 * S;
        while t <= 300 * S {
            let d = reconcile(&s, &mut m, t, 8, &idle);
            assert_eq!((d.desired, d.stabilized), (8, Some(8)), "t = {}", t / S);
            t += 15 * S;
        }
        // t = 315: those of t = 15 are not after the cutoff any more: 0, bounded by
        // minReplicas (100 % per 15 s would allow removing all)
        let d = reconcile(&s, &mut m, 315 * S, 8, &idle);
        assert_eq!(
            (d.desired, d.stabilized, d.reason),
            (1, Some(0), "TooFewReplicas")
        );
        // memory stays bounded: at most one recommendation per sync inside the window, plus one
        assert!(
            m.recommendations.len() <= 300 / 15 + 2,
            "{}",
            m.recommendations.len()
        );
    }

    #[test]
    fn behavior_policies_limit_the_rate() {
        let s = spec(Some(default_behavior()));
        let mut m = HpaMemory::default();
        // 1 pod at 1000 %: 20 -> up limit max(1 + 4, ceil(1 * 2)) = 5
        let d = reconcile(&s, &mut m, 15 * S, 1, &ready(&[5000]));
        assert_eq!((d.desired, d.reason), (5, "ScaleUpLimit"));
        // 15 s later the event (at t = 15, change 4) is not after the cutoff (t - 15 s): the
        // period starts at 5 again: max(9, 10) = 10 = maxReplicas
        let d = reconcile(&s, &mut m, 30 * S, 5, &ready(&[5000; 5]));
        assert_eq!((d.desired, d.reason), (10, "TooManyReplicas"));
        // a 60 s period counts both scale-ups: start = 10 - 5 - 4 = 1
        let mut b = default_behavior();
        b.up.policies = vec![ScalingPolicy {
            kind: PolicyKind::Pods,
            value: 2,
            period_ns: 60 * S,
        }];
        let s2 = spec(Some(b));
        let mut m2 = HpaMemory::default();
        let d = reconcile(&s2, &mut m2, 15 * S, 1, &ready(&[5000]));
        assert_eq!(d.desired, 3);
        let d = reconcile(&s2, &mut m2, 30 * S, 3, &ready(&[5000; 3]));
        assert_eq!(
            (d.desired, d.reason),
            (3, "ScaleUpLimit"),
            "2 per 60 s already used"
        );
        let d = reconcile(&s2, &mut m2, 76 * S, 3, &ready(&[5000; 3]));
        assert_eq!(d.desired, 5, "the event of t = 15 left the period");
        // selectPolicy Min picks the smaller change; Disabled none
        let mut b = default_behavior();
        b.up.select = Select::Min;
        let mut m3 = HpaMemory::default();
        let d = reconcile(&spec(Some(b.clone())), &mut m3, 15 * S, 1, &ready(&[5000]));
        assert_eq!(d.desired, 2, "min(1 + 4, ceil(2)) = 2");
        b.up.select = Select::Disabled;
        let mut m4 = HpaMemory::default();
        let d = reconcile(&spec(Some(b)), &mut m4, 15 * S, 1, &ready(&[5000]));
        assert_eq!(d.desired, 1);
    }

    #[test]
    fn scale_down_policies() {
        let mut b = default_behavior();
        b.down.stabilization_ns = 0;
        b.down.policies = vec![
            ScalingPolicy {
                kind: PolicyKind::Percent,
                value: 50,
                period_ns: 60 * S,
            },
            ScalingPolicy {
                kind: PolicyKind::Pods,
                value: 1,
                period_ns: 60 * S,
            },
        ];
        let s = spec(Some(b));
        let mut m = HpaMemory::default();
        // 10 pods idle, desired 1 (min): Max picks the largest removal: min(int(10 * 0.5), 9) = 5
        let d = reconcile(&s, &mut m, 15 * S, 10, &ready(&[0; 10]));
        assert_eq!((d.desired, d.reason), (5, "ScaleDownLimit"));
        // within 60 s: period start = 5 + 5 = 10: min(5, 9) = 5: no further removal
        let d = reconcile(&s, &mut m, 30 * S, 5, &ready(&[0; 5]));
        assert_eq!(d.desired, 5);
        // after the period: int(5 * 0.5) = 2
        let d = reconcile(&s, &mut m, 80 * S, 5, &ready(&[0; 5]));
        assert_eq!(d.desired, 2);
    }

    #[test]
    fn min_and_max_bypass_the_calculation() {
        let s = spec(Some(default_behavior()));
        let mut m = HpaMemory::default();
        let d = reconcile(&s, &mut m, 15 * S, 12, &ready(&[0; 12]));
        assert_eq!((d.desired, d.reason, d.calc), (10, "AboveMax", None));
        let mut s = s;
        s.min_replicas = 3;
        let d = reconcile(&s, &mut m, 30 * S, 2, &ready(&[0; 2]));
        assert_eq!((d.desired, d.reason), (3, "BelowMin"));
        // the calculation's result is bounded by min and max
        let d = reconcile(&s, &mut m, 45 * S, 3, &ready(&[0; 3]));
        assert_eq!(d.desired, 3);
        let d = reconcile(&s, &mut m, 60 * S, 9, &ready(&[5000; 9]));
        assert_eq!((d.desired, d.reason), (10, "TooManyReplicas"));
    }

    #[test]
    fn per_direction_tolerances() {
        let mut b = default_behavior();
        b.up.tolerance = Some(0.01);
        b.down.tolerance = Some(0.05);
        let s = spec(Some(b));
        let t = s.tolerances();
        assert!(t.within(1.005) && !t.within(1.02) && t.within(0.96) && !t.within(0.94));
    }

    #[test]
    fn starting_pods() {
        let mut r = Replicas::default();
        // 2 ready, desired 5, start-up 20 s: 3 start, nothing changes now
        let p = r.rescale(2, 5, 10 * S, 20 * S);
        assert_eq!((p.started, p.cancelled, p.goal), (3, 0, None));
        assert_eq!(r.next(), Some(30 * S));
        // desired 3 at t = 15: two starting pods are deleted, no ready one
        let p = r.rescale(2, 3, 15 * S, 20 * S);
        assert_eq!((p.cancelled, p.goal), (2, None));
        // desired 1: the last starting pod and one ready pod go
        let p = r.rescale(2, 1, 16 * S, 20 * S);
        assert_eq!((p.cancelled, p.goal), (1, Some(1)));
        assert_eq!(r.due(40 * S), 0);
        // no start-up delay: the goal at once
        let p = r.rescale(1, 4, 50 * S, 0);
        assert_eq!((p.started, p.goal), (3, Some(4)));
        let _ = r.rescale(4, 6, 60 * S, 5 * S);
        let _ = r.rescale(4, 7, 62 * S, 5 * S);
        assert_eq!(r.due(65 * S), 2);
        assert_eq!(r.due(67 * S), 1);
        assert!(r.pending.is_empty());
    }
}
