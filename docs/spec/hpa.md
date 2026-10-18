# HPA autoscaling

SimOxide models Kubernetes' horizontal pod autoscaler (HPA, `autoscaling/v2`) with a CPU
utilisation target, scaling a target group of the model's scaling policy definition (SPD). The
controller is a port of Kubernetes' arithmetic. The structural change of a rescale is
Slingshot's SPD adjustment: an absolute adjustment of the group through `MainTransformation.qvto`
([Scaling policies](../guide/spd.md#adjustments)). These rules say what SimOxide models, what it
simplifies, and why. The configuration and the command line are in the
[HPA guide](../guide/hpa.md).

Sources (Kubernetes `master`, October 2026):

| Tag | Source |
|---|---|
| [T] | <https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/> |
| [A] | <https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/horizontal-pod-autoscaler-v2/> |
| [K] | <https://kubernetes.io/docs/reference/command-line-tools-reference/kube-controller-manager/> (`--horizontal-pod-autoscaler-*`) |
| [H] | `pkg/controller/podautoscaler/horizontal.go` |
| [R] | `pkg/controller/podautoscaler/replica_calculator.go` |
| [U] | `pkg/controller/podautoscaler/metrics/utilization.go`, `metrics/client.go` |
| [D] | `pkg/apis/autoscaling/v2/defaults.go` |

The code is in `crates/simoxide-spd/src/hpa/`: `controller.rs` (the arithmetic, plain data),
`run.rs` (the controller in a run), `config.rs` (the configuration file).

## HPA-1 Pods, target and metric

- **HPA-1.1** An autoscaler scales one SPD target group (`scaleTargetRef`, by name or id).
  - Elastic infrastructure: the pods are the group's containers.
  - Service group: the pods are its assembly contexts. Each is measured on the container it is
    allocated on.
  - Competing-consumer groups are not supported.
  - At most one autoscaler per group. SPD policies on the same group are allowed; their
    adjustments change the pods the HPA sees (as two controllers of one Deployment would).
- **HPA-1.2** The replica count `currentReplicas` is the scale's `spec.replicas` [H]: the ready
  pods (the group's elements) plus the starting ones (HPA-6). Replicas never drop below one
  (`minReplicas >= 1`). Kubernetes scales to zero only on object or external metrics [T],
  which SimOxide does not model.
- **HPA-1.3** The metric is the CPU's average utilisation relative to the pods' requests
  (`type: Resource`, `name: cpu`, `target.type: Utilization`), the only metric supported. Without
  `metrics` the target is 80 % (`autoscaling.DefaultCPUUtilization`, [D]).
  - A pod's CPU is the container's processing resources of type `CPU`.
  - The request is their cores (`numberOfReplicas`): a Guaranteed pod whose request equals its
    limit. `simoxide.cpuRequest` sets another request in cores; then the utilisation can exceed
    100 %.

## HPA-2 The sync

- **HPA-2.1** Every autoscaler is reconciled once per sync period (`syncPeriod`, default 15 s,
  `--horizontal-pod-autoscaler-sync-period` [K]). The first sync is one period after the start
  (`simoxide.firstSync`), so that the first metric window is full.
- **HPA-2.2** `reconcileAutoscaler` [H], in this order:
  1. The first sync records `currentReplicas` as a recommendation
     (`recordInitialRecommendation`).
  2. If `currentReplicas > maxReplicas`, desired is `maxReplicas`; if it is below `minReplicas`,
     desired is `minReplicas`. Neither path computes metrics, stabilises or rate-limits.
  3. Otherwise desired is the replica calculator's count (HPA-3), stabilised and rate-limited
     (HPA-5).
  4. A calculator error (no ready pod with a metric) changes nothing and records nothing.
- **HPA-2.3** If desired differs from `currentReplicas`, the autoscaler rescales (HPA-6) and, with
  a `behavior`, records the scale event (`storeScaleEvent`).
- **HPA-2.4** The sync is an external note of the event list (FIFO among same-time events, as
  SPD time events). A rescale's adjustment is queued at the current time, behind the events
  already queued, as an SPD policy's request (`ModelAdjustmentRequested`).

## HPA-3 The replica calculator

`GetResourceReplicas` [R] with `GetResourceUtilizationRatio` [U]:

- **HPA-3.1** Pods are grouped (`groupPods`): ready pods with a metric, unready pods (HPA-4.3), and
  missing pods (no metric). Removed pods (drained containers) are no longer group elements:
  Kubernetes ignores terminating pods.
- **HPA-3.2** `utilization = ⌊100 · Σ usage / Σ request⌋` over the ready pods, in integer
  milli-cores. `ratio = utilization / target`.
- **HPA-3.3** If no pod is unready while scaling up and none is missing: within the tolerance
  (`1 - tol ≤ ratio ≤ 1 + tol`, inclusive) the count stays; otherwise it is
  `⌈ratio · readyPods⌉`.
  - The tolerance defaults to 0.1 (`--horizontal-pod-autoscaler-tolerance` [K]).
  - `behavior.scaleUp.tolerance` and `scaleDown.tolerance` set per-direction tolerances (the
    `HPAConfigurableTolerance` fields [A], [H]).
  - The multiplier is the number of ready pods, not `currentReplicas` as [T]'s formula has it.
- **HPA-3.4** Otherwise the ratio is computed again over more pods:
  - unready pods at 0 % if scaling up;
  - missing pods at `max(100, target)` % of their request if scaling down, and at 0 % if scaling
    up.

  If the new ratio is within the tolerance or crosses 1 against the first one, the count stays.
  Otherwise it is `⌈newRatio · pods counted⌉`, unless that would move against the direction.
- **HPA-3.5** No ready pod with a metric is an error: "did not receive metrics for targeted pods"
  (HPA-2.2).

## HPA-4 The CPU metric

- **HPA-4.1** A pod's usage is its CPU's mean busy cores over the metric window that ends at the
  sync.
  - The window is `simoxide.metricWindow`, default the sync period: metrics-server reports usage
    over its resolution of 15 s.
  - The engine integrates each tracked resource's busy fraction, the value its overall
    utilisation probe reports: busy instances over instances, and Slingshot's busy cores over
    cores. This needs no monitor in the model (`Simulation::track_busy`, `busy_integral`).
  - A note one window before each sync reads the integrals; the sync reads them again.
- **HPA-4.2** The usage is rounded as Kubernetes rounds it: nanocores to milli-cores, rounded up
  (`Quantity.MilliValue()` [U]).
- **HPA-4.3** Unready pods are those `groupPods` sets aside for CPU [R]:
  - starting pods (HPA-6): `Pending`, or not Ready;
  - pods inside the CPU initialisation period (`cpuInitializationPeriod`, default 5 min [K])
    that became ready less than one metric window before the sync (the metric's timestamp is
    before the Ready transition plus the window).

  Past the initialisation period, a pod's metric covers the part of the window since it became
  ready. The initial-readiness delay (30 s [K]) never matters: SimOxide's pods never turn
  unready again.
- **HPA-4.4** The pods of the start are ready (started long before).

## HPA-5 Stabilisation and rate limits

Without `spec.behavior` the controller takes its legacy path, as Kubernetes does: the API
server fills `behavior` only when one is given [D].

- **HPA-5.1** Stabilisation, which records the unstabilised recommendation:
  - Without `behavior` (`stabilizeRecommendation` [H]): the maximum of the recommendations of
    the last `downscaleStabilization` (default 5 min [K]) and this one, in both directions.
  - With `behavior` (`stabilizeRecommendationWithBehaviors` [H]): `min(max(current, up),
    down)`. Here `up` is the minimum over the scale-up window (default 0 s: no stabilisation)
    and `down` the maximum over the scale-down window (default: the controller's 5 min).
  - Windows are strict: a recommendation counts if it is newer than `now - window`.
  - A recommendation older than every window is replaced rather than appended.
- **HPA-5.2** Rate limits:
  - Without `behavior` (`convertDesiredReplicasWithRules`): scale up to at most
    `max(2 · current, 4)`, within `[minReplicas, maxReplicas]`.
  - With `behavior` (`convertDesiredReplicasWithBehaviorRate`), per policy:
    - `periodStart = current - added + deleted`, the scale events newer than the policy's
      period;
    - `Pods`: `periodStart ± value`;
    - `Percent`: `⌈periodStart · (1 + v/100)⌉` up, `⌊periodStart · (1 - v/100)⌋` down;
    - `selectPolicy` `Max` takes the largest change, `Min` the smallest, `Disabled` none.
  - Default policies [D]: up, the larger of 4 pods and 100 % per 15 s; down, 100 % per 15 s.
  - The limit never goes past the current count. `maxReplicas` and `minReplicas` bound the
    result.
- **HPA-5.3** Scale events are kept for the longest policy period of their direction. An
  outdated event is overwritten.

## HPA-6 Pods starting and leaving

- **HPA-6.1** A scale-up creates pods at once. They count as replicas (HPA-1.2) and are unready
  (HPA-4.3). They take requests only once ready, after `simoxide.startupDelay` (default 0):
  Kubernetes routes Service traffic to ready endpoints only.
  - At the ready time a note joins them: the group's absolute adjustment to its ready count plus
    the pods now ready. The new containers, assemblies, allocations, load-balancer branches and
    cloned monitors are Slingshot's, as for an SPD policy.
  - With a delay of 0 the adjustment runs at the rescale.
- **HPA-6.2** A scale-down deletes pods in the ReplicaSet controller's order:
  - starting pods first, newest first;
  - then ready pods, through the absolute adjustment, which removes the newest elements.

  Removed containers drain by default ([Model changes](./model-changes.md) MC-8): their requests
  in progress finish, including the calls they still make to other services (a terminating
  pod's graceful shutdown), and new requests go elsewhere, as a terminating pod's endpoints
  leave the Service.
- **HPA-6.3** The adjustment writes the desired count into the autoscaler's inactive SPD policy
  (`_simoxide_hpa_<name>`, an `AbsoluteAdjustment`) and enacts it. A group-size constraint of
  the SPD still clamps it. The outcome changes no SPD constraint state. A failed adjustment is
  counted, and the next sync sees the group as it is.

## HPA-7 State, forks, snapshots, changes

- **HPA-7.1** The controller's state is plain data, part of forks and of snapshots (section
  `hpa`). It holds, per autoscaler:
  - the spec, the recommendations and the scale events;
  - the starting pods, the ready pods with their start and ready times;
  - the window readings and the counters.

  A fork or a restored snapshot continues like the uninterrupted run. Restoring an HPA
  snapshot needs the same configuration.
- **HPA-7.2** At a resume, parameters change from then on: `minReplicas`, `maxReplicas`,
  `averageUtilization`, `tolerance`, `startupDelay`, the stabilisation windows. They are set
  with `--hpa-set` or with `hpa` lines of a change file (`SpdRun::hpa_set`). The memory of
  recommendations and events stays.

## HPA-8 Bounds

- **HPA-8.1** The controller's state is bounded:
  - recommendations: at most one per sync inside the longest window, plus one;
  - scale events: at most one per sync inside the longest policy period, plus one;
  - starting and ready pods: at most `maxReplicas` each;
  - readings: at most one per CPU of a ready pod;
  - adjustment requests: the request table empties whenever no request is queued.
- **HPA-8.2** Every pod created stays in the run's model after it is removed. The change path
  keeps typed ids and resource indices stable: a removed container is a tombstone of about 22
  graph objects with its cloned monitors, one resource and about 8 KB in a snapshot. Every
  later model change rebuilds them as well. The growth is linear in the pods created; a run
  without rescales does not grow.
- **HPA-8.3** `controller.maxCreatedPods` (default 20 000 over all autoscalers) stops a run that
  creates more, with an error.

## What SimOxide simplifies

| Kubernetes | SimOxide | Why |
|---|---|---|
| metrics-server scrapes every 15 s; the HPA reads the latest scrape, up to one resolution old | the metric window ends at the sync | no scrape phase to model; lag can be added as a later option |
| Pod lifecycle: scheduling, image pull, container start, readiness probe | one start-up delay, then Ready | the effects on scaling are that new pods count as replicas, are unready and take no traffic, all modelled |
| Ready pods can turn unready (failing probes) | never | no failure model |
| ReplicaSet deletion ranking (node co-location, deletion cost, log-scaled ages) | starting pods newest first, then the newest ready pods | the Slingshot transformation removes the newest elements |
| Several metrics (max over metrics), memory, Pods/Object/External metrics | CPU utilisation only | the simulation measures CPU; others could follow (response time as an External metric) |
| Scale to zero (`HPAScaleToZero`) | `minReplicas >= 1` | resource metrics cannot scale to zero [T] |
| Controller resync jitter, API update conflicts | exact periods, instant updates | deterministic runs |
