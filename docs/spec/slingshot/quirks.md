# Slingshot quirks

Every known Slingshot bug or oddity that changes results, with its observable effect. Decision U5:
the Slingshot flavour **fixes** each one by default; a compat flag per quirk reproduces it; the
Slingshot-exact mode turns all flags on, because its oracle (the `det` runner) is Slingshot with all
of them.

"Fixed behaviour" says what the flavour does without the flag. It follows decision U13: where
Slingshot's behaviour is a bug, the fixed behaviour is **SimuLizar's (SimOxide's) semantics**
(asynchronous forks run, delay resources work, no handler is disposed, demands are served one after
the other, monitors are honoured, …). For SPD quirks, which have no SimuLizar counterpart, it is the
evident intent of Slingshot's code or of the SPD metamodel. *defined* means this page fixes it;
*proposed (phase)* gives the direction and leaves the details to that phase; *kept* means the quirk
is Slingshot's design and the fixed behaviour equals it (the flag exists for completeness).

Behaviour that is Slingshot's design rather than a bug is not listed here and has no flag: double
time and FIFO cascades (SS-ENG, SS-BUS), the stop at `simTime` without drain (SS-RUN-3/4), zero
demands as JIFFY jobs (SS-RES-2.2), FCFS without lost time (SS-RES-3.4), instantaneous utilisation
(SS-MEAS-3.3), first allocation match (SS-ACT-9), `Math.random()` branches (SS-RNG-1.2), links
always simulated (SS-ACT-7.2), the release that does not wait (SS-ACT-8.3).

## Overview

| # | Quirk | Effect | Fixed behaviour | Evidence |
|---|---|---|---|---|
| Q1–Q16 | SPD interpreter and adjustment | [below](#spd-quirks-q1-q16) | evident intent of the SPD code (below) | `DESIGN-SPD.md` §1 |
| Q17 | several demands of one InternalAction | concurrent demands, cursor advanced per demand, skipped/overlapping actions, then a disposed handler | defined: the demands are served one after the other, in list order, as SimuLizar (ACT-3.1); the action completes after the last | `ss_two_demands` |
| Q18 | a throwing handler is disposed, the run goes on | one bad request freezes a stage for every user; the run "succeeds" | defined: the run aborts with the error at that event (as SimuLizar) | SS-BUS-7; 5 probes, 8 corpus models |
| Q19 | loop count 0 | SEFF: body once, then exception; usage: exception | defined: count ≤ 0 skips the body (ACT-7.1, WL-4.4) | `ss_loop_zero`, `ss_usage_loop_zero` |
| Q20 | roles resolved by role id alone | entry calls go to the first assembly of the component; external calls use the first connector with that required role | defined: the delegation connector's assembly; the requiring assembly's connector (ACT-2.1, ACT-2.6) | `ss_role_lookup` |
| Q21 | guarded branch without a true guard | exception, disposed handler | defined: run error (as SimuLizar ACT-6.2) | `ss_guard_none` |
| Q22 | CollectionIteratorAction | INNER drawn once; frame never popped; `UserRequestFinished` precondition fails | defined: ACT-7.2/7.3 (frame per iteration, INNER per iteration, popped) | corpus `h10`, `h27` |
| Q23 | ForkAction | async ignored; sync children share a rotating cursor and throw; no synchronisation point throws | defined: as SimuLizar: asynchronous behaviours run detached (ACT-9); one cursor (and frame chain) per synchronous child, join | `ss_async_fork`; corpus `h11`, `h12`, `h26`, `x_pem_fork` |
| Q24 | Acquire/Release need a ParametricResourceDemand | exception, disposed handler | defined: one unit when there is none (ACT-8.2); with a demand, its amount (Slingshot design) | corpus `h13`, `h28` |
| Q25 | no delay resources | every demand on a Delay resource aborts the user | defined: SimuLizar's delay scheduler | `ss_delay_resource`; corpus `h04`, `h25`, `x_pem_fork` |
| Q26 | stale PS completions sample the state | extra state and utilisation tuples | defined: stale events record nothing (as SimuLizar, which cancels them) | `ss_fifo_ties`, `ss_ps_multicore` |
| Q27 | passive capacity without a frame | a capacity that uses a component parameter aborts the run before t = 0 | defined: evaluated in the component instance's frame (ACT-8.1) | corpus `x_pem_acquire` |
| Q28 | link failure check | a `Math.random()` draw per transfer even at probability 0; an aborted transfer stalls the link | defined: draw only if probability > 0; the next queued transfer starts | `ss_link` (draws) |
| Q29 | response-time monitors ignored | ELSC, system-operation and external-call points record nothing | defined: honoured as SimuLizar (MEAS-2 to MEAS-4) | `ss_monitors`; `x_ss_minimal` |
| Q30 | composite components and subsystems | entry into one stops the user silently; entered through a connector, its inner assemblies have no allocation and every demand aborts (abort handler disposed) | defined: ACT-2.2, inner assemblies allocated with the composite (ACT-2.8) | corpus `h15`, `x_pem_subsystem`, `x_pem_replicated_composite`, `x_pem_simple_optimisation_composites` |
| Q31 | infrastructure calls break the assembly-operation monitor | no assembly-operation response time after the first infrastructure call | defined: infrastructure SEFFs are not measured, nothing is disposed | corpus `h18` |
| Q32 | `simTime` the only stop condition, no livelock guard | zero-time loops never end; maximum measurement count ignored | defined: the maximum measurement count is honoured as in SimuLizar (SIM-6); SimOxide's livelock guard (always on, also with the flag) | corpus `x_pem_fork` |
| Q33 | entry-level assembly-operation key uses the system role | a measuring point with the component's provided role never matches the entry call | defined: the inner (component) role | `ss_role_lookup` |
| Q34 | probes per specification / per passive resource | a monitor for replica *k* gets replica-independent values; passive instances of several assemblies share series | defined: passive series per (passive resource, assembly context) as SimuLizar; an active-resource point with replica *k* gets the per-core values of core *k* ([compat flags](#compat-flags), defined in S2) | `ss_ps_multicore` |
| Q35 | silent stalls | unsupported actions, `ResourceCall`s, missing connectors or SEFFs end a request without an error | defined: run error; `ResourceCall` as ACT-3.6 | corpus `h23`, `x_pem_reliability` (RecoveryAction: no request completes) |
| Q36 | infrastructure-call inputs ignored | no input frame for the callee | defined: ACT-3.5 | corpus `h18` |
| Q37 | calls between unconnected containers | `IllegalArgumentException` disposes `onCallOverWireRequested`: no further transfer anywhere | defined: run error (as SimuLizar ACT-11.2), except when a side was removed from the environment by a model change (a draining container's requests in progress): then a link touching the other side, as Slingshot's code intends (P3) | – |

## The exact mode {#exact-mode}

The Slingshot-exact mode (`--flavour slingshot-exact`) runs every flag, Q17 and
Q18 included, by emulating Slingshot's handlers: each quirk that throws ("up to the throw" below)
throws in the same handler with the same exception class, and the handler is disposed with the
side effects it had made (draws, frames pushed or popped, cursor moves, measurements). Checked on
the 22 Slingshot corpus cases with handler errors plus two Q37 cases
([SS-X-8](./exact.md#ss-x-8-the-slingshot-exact-corpus); `crates/simoxide-sim/tests/slingshot_exact.rs`,
`handler_errors_match_the_reference`). The SPD quirks Q1–Q16 are the SPD interpreter's, all on in this mode.

## Compat flags in SimOxide {#compat-flags}

SimOxide's Slingshot flavour (`--flavour slingshot`, here called the fast mode; [Slingshot-compatible mode](../../guide/slingshot.md))
has one flag per quirk: `--quirks Q26,Q29`, `--quirks fast` (every flag the fast mode supports),
`--quirks list`; in the library `simoxide_sim::Quirks` / `Quirk::Q26` in
`FlavourConfig::slingshot(..)`. Without a flag the fixed behaviour of the table above applies.
"Fast mode" says how far the fast mode reproduces the quirk under its flag:

| # | Fast mode | Notes |
|---|---|---|
| Q1–Q16 | SPD | the SPD interpreter's flags (`simoxide_spd::Quirks`, `simoxide run --spd-quirks`; Q5–Q8, Q14, Q16 act through the adjustment); `simoxide_sim::Quirks` carries the same numbers so that one set drives both |
| Q17 | refused | concurrent demands with a shared cursor need the exact mode's handler steps; refused for models with an internal action with two or more demands. The exact mode (`--flavour slingshot-exact`) reproduces it |
| Q18 | refused | handler disposal is the exact mode's: there every handler has a disposed bit, the throwing handler keeps the side effects up to the throw and never runs again |
| Q19 | up to the throw | SEFF loop count 0: body once, then a run error; negative count and usage-loop count ≤ 0: run error at the loop |
| Q20 | full | entry calls: the first system assembly context whose component provides the inner role (by id); external calls: the first system assembly connector with the required role (by id) |
| Q21 | up to the throw | the same run error as the fixed behaviour |
| Q22 | up to the throw | INNER drawn once, the frame left on the stack (with Slingshot's frame misalignment), run error at the end of the entry-level call |
| Q23 | up to the throw | asynchronous behaviours ignored; no synchronisation point: run error; one synchronous behaviour runs on the user's stack; two or more: run error at the fork |
| Q24 | up to the throw | run error at an Acquire/Release without demand |
| Q25 | full | Delay resources (and any other non-FCFS/PS policy) have no resource: the user is aborted |
| Q26 | full | the superseded completion stays in the event list at its old position and samples state and utilisation |
| Q27 | full | capacities evaluated without a frame (an unknown variable is a run error before t = 0) |
| Q28 | full | a `Math.random()` draw at every link completion; a failed transfer stalls the link |
| Q29 | full | no series for these measuring points |
| Q30 | up to the throw | entry into a composite stalls the user; inner assembly contexts unallocated, a demand there is a run error |
| Q31 | up to the throw | run error at the first infrastructure call when an assembly-operation response-time monitor exists |
| Q32 | full | the maximum measurement count is ignored (`simTime` must be set); the livelock guard stays on |
| Q33 | full | the entry-level SEFF's assembly-operation key uses the system's role |
| Q34 | full | every replica's monitor gets the resource totals; passive monitors merged per passive resource |
| Q35 | full | the request stalls (the process never continues); resource calls are skipped |
| Q36 | full | the callee gets the shared empty frame |
| Q37 | up to the throw | a link touching either container is used; none: run error |

"Up to the throw" means: Slingshot throws in a handler at that point, disposes the handler and runs
on (Q18); the fast mode stops there with a run error. *Provisional:* the fixed behaviours marked
*proposed* (Q4, Q6, Q7, Q9, Q13, Q14) and *kept* (Q3) above are not confirmed by the user; the SPD
phases implement them as proposed. The fixed behaviour of Q34 for active resources is defined by
S2: a measuring point with replica *k* of a processor-sharing resource with several cores records
core *k* (the jobs `reportCoreUsage` attributes to it, utilisation 1 or 0), FCFS and delay
resources and single-core resources the resource itself; a replica outside the resource records
nothing.

## Details

### SS-Q17 Several demands of one InternalAction {#ss-q17}

[SS-ACT-3.1/3.2](./seff.md#ss-act-3-internalaction). All `ParametricResourceDemand`s start at once and
every completion advances the shared cursor. Probe `ss_two_demands` (CPU 1.0 and HDD 2.0, then 0.5 and
0.25 on CPU): the request ends at 1.75 while the HDD still serves; the HDD completion at 2.0 throws a
`NullPointerException` in `onActiveResourceFinished`; afterwards no demand completion advances any SEFF.
Fixed (U13): the demands are served one after the other, as SimuLizar (ACT-3.1): each demand is
requested when the previous one has completed, and the cursor advances once, after the last. Under the
flag all demands start at once and every completion advances the cursor.

### SS-Q18 A throwing handler is disposed {#ss-q18}

[SS-BUS-7](./core.md#ss-bus-the-event-bus). Compat: log the error, drop that handler for the rest of
the run, continue. Fixed: a `SimError` at the event, with the exception's message, as SimOxide does for
reference aborts. Every quirk below that "disposes a handler" goes through this one: with Q18 fixed,
those runs end with an error at the first occurrence.

### SS-Q19 Loop count 0 {#ss-q19}

SEFF ([SS-ACT-6.2](./seff.md#ss-act-6-loops-and-collection-iterators)): the body runs once, then
`NoSuchElementException`. Usage model ([SS-WL-4.3](./usage.md#ss-wl-4-usage-actions)): the context
builder throws before the body. A negative SEFF count fails a precondition.

### SS-Q20 Roles resolved by role id {#ss-q20}

[SS-ACT-1.2](./seff.md#ss-act-1-entry-level-system-calls), [SS-ACT-2.2](./seff.md#ss-act-2-external-calls-and-role-lookup).
Harmless when every component is assembled once (all Slingshot examples); wrong as soon as a
component is assembled twice. SPD's load balancer adds a *new* required role per replica, so SPD
scale-outs are not affected.

### SS-Q21 Guarded branch without a true guard {#ss-q21}

[SS-ACT-5.3](./seff.md#ss-act-5-branches). SimuLizar aborts the run too, so the fixed behaviour is
SimuLizar's.

### SS-Q22 CollectionIteratorAction {#ss-q22}

[SS-ACT-6.3](./seff.md#ss-act-6-loops-and-collection-iterators). After the first request with a
collection iterator finishes its SEFF, `UserRequestFinished` is disposed; no user completes a request
afterwards (corpus `h10`: one error, then only resource rows).

### SS-Q23 ForkAction {#ss-q23}

[SS-ACT-10](./seff.md#ss-act-10-forkaction). Slingshot's fork works only with exactly one synchronous
behaviour. Fixed (U13): as SimuLizar: each synchronous behaviour gets its own cursor (and its own frame
chain on a copied stack), the parent continues once when all have finished; asynchronous behaviours
run detached (ACT-9); a fork without a synchronisation point is a fork of asynchronous behaviours
only. Under the flag: asynchronous behaviours ignored, one rotating cursor, the exceptions.

### SS-Q24 Acquire/Release without ParametricResourceDemand {#ss-q24}

[SS-ACT-8.1](./seff.md#ss-act-8-acquire-and-release). Most Palladio example models (refsim's corpus
included) have no demand on their acquire actions.

### SS-Q25 No delay resources {#ss-q25}

[SS-RES-1](./resources.md#ss-res-1-which-resources-exist). Combined with Q32 and a zero think time this
is a livelock at t = 0 (`x_pem_fork`).

### SS-Q26 Stale PS completions sample the state {#ss-q26}

[SS-RES-2.6](./resources.md#ss-res-2-processor-sharing). Each arrival and each completion of another job
leaves one stale `JobProgressed`; at its due time it records the unchanged state and utilisation. With
the flag off, those tuples are not recorded (the events can still be used internally in the exact
mode's cascade).

### SS-Q27 Passive capacity without a frame {#ss-q27}

[SS-RES-4.1](./resources.md#ss-res-4-passive-resources).

### SS-Q28 Link failure check {#ss-q28}

[SS-RES-5.3](./resources.md#ss-res-5-linking-resources). The draw at probability 0 shifts the
`Math.random()` stream, so branch draws after a transfer differ from a model without links. The stall
after a failure needs `failureProbability > 0` (not in any corpus model).

### SS-Q29 Response-time monitors ignored {#ss-q29}

[SS-MEAS-2.2](./monitoring.md#ss-meas-2-which-monitors-are-honoured). With the flag off the flavour
records the SimuLizar measuring points it can map (entry-level call, system operation, external call),
with Slingshot's timing (passed elements).

### SS-Q30 Composite components and subsystems {#ss-q30}

[SS-ACT-1.5](./seff.md#ss-act-1-entry-level-system-calls).

### SS-Q31 Infrastructure calls break the assembly-operation monitor {#ss-q31}

[SS-MEAS-2.4](./monitoring.md#ss-meas-2-which-monitors-are-honoured).

### SS-Q32 `simTime` is the only stop condition {#ss-q32}

[SS-RUN-5](./core.md#ss-run-start-and-stop). Fixed (U13): the maximum measurement count of the run
configuration stops the run as in SimuLizar (SIM-6), next to `simTime`; the flag ignores it. SimOxide's
livelock guard ([Deviations](../../correctness/deviations.md)) applies in all modes, flag or not; an
exact comparison of such a model is impossible anyway (the oracle never ends).

### SS-Q33 Entry-level assembly-operation key {#ss-q33}

[SS-MEAS-2.3](./monitoring.md#ss-meas-2-which-monitors-are-honoured).

### SS-Q34 Probes per specification and per passive resource {#ss-q34}

[SS-MEAS-3.1](./monitoring.md#ss-meas-3-active-resources), [SS-MEAS-4.1](./monitoring.md#ss-meas-4-passive-resources).

### SS-Q35 Silent stalls {#ss-q35}

[SS-ACT-4.3](./seff.md#ss-act-4-start-stop-and-the-end-of-a-behaviour),
[SS-ACT-3.4](./seff.md#ss-act-3-internalaction), [SS-WL-6.2](./usage.md#ss-wl-6-not-supported-or-not-loaded).

### SS-Q36 Infrastructure-call inputs ignored {#ss-q36}

[SS-ACT-2.5](./seff.md#ss-act-2-external-calls-and-role-lookup).

### SS-Q37 Calls between unconnected containers {#ss-q37}

[SS-ACT-7.1](./seff.md#ss-act-7-calls-over-the-wire).

## SPD quirks Q1–Q16 {#spd-quirks-q1-q16}

Defined in `DESIGN-SPD.md` §1.5, rules on [SPD](./spd.md). The fixed behaviour is the evident intent
of the code (U13); *proposed* rows are confirmed in phases P1/P2 (interpreter, adjustment). Fixes
inside the QVTo transformations (Q4, Q6, Q7) need a patched copy of the transformations, selected
when the flag is off; the unchanged transformations run under the flag.

| # | Quirk | Rule | Fixed behaviour |
|---|---|---|---|
| Q1 | `doOnDisregard` does not restore the state | SS-SPD-3.2 | defined: a disregard restores the state as `doOnDisregard` intends (no effect on results known). P1: `Quirks::q1_disregard_keeps_state`. |
| Q2 | constraint state changes when the request is made, even if QVTo fails | SS-SPD-3.2 | defined: the adjustor's state change is committed only by a successful `ModelAdjusted`. P1: `Quirks::q2_commit_on_request`. |
| Q3 | AVERAGE is an EMA (60 s, 10 s, 0.2); the others use the last 10 values | SS-SPD-2.4 | kept: Slingshot's aggregator design (no SimuLizar counterpart, no other intent in the code); P1 may revisit. P1: kept, no flag (*provisional*). |
| Q4 | relative 0 % ignores `minAdjustmentValue` | SS-SPD-4.2 | proposed (P2): a zero change stays zero; otherwise the magnitude is at least `minAdjustmentValue`, in the direction of the percentage. P1: the arithmetic and `Quirks::q4_relative_zero` (used by the Q9 check); P2: the patched `AdjustmentCalculator.qvto` (`crates/simoxide-reconf/transformations/spd-fixed`), *provisional*. |
| Q5 | scale-in aborts one job, loses the others | SS-SPD-5.4 | defined in the design (§5.3): drain |
| Q6 | every link connects every container after an adjustment | SS-SPD-4.2 | proposed (P2): a new container joins the links of the unit container (appended); other links unchanged; removed containers leave every link. P2: `Quirks::q6_all_links`, the patched `ResourceEnvironmentTransformation.qvto`, *provisional* |
| Q7 | top-down scale-in removes emptied containers one enactment late | SS-SPD-4.2 | proposed (P2): removed in the same enactment (the transformation's own TODO). P2: `Quirks::q7_late_removal`, the patched `TopDownTransformationInfrastructure.qvto`, *provisional* |
| Q8 | top-down placement `->any(true)`: Java-hash order | SS-SPD-4.2 | the QVTo engine's deterministic insertion order (documented deviation, `DESIGN-SPD.md` §5.5); the Slingshot-exact mode uses QVTo's ordered mode, which equals the `det` runner's insertion order |
| Q9 | only thrashing constraints in the chain | SS-SPD-1.4 | proposed (P1): the `TargetGroupSizeConstraint` is also checked in the chain, so a policy whose change would be clamped to zero is disregarded (and changes no constraint state). P1: `Quirks::q9_no_size_check`, *provisional*. |
| Q10 | static fields in `TargetGroupUtils` | – | not reproduced (no shared JVM state in SimOxide) |
| Q11 | unsupported stimuli: NPE at set-up | SS-SPD-2.2 | defined: a configuration error naming the stimulus. P1: `Quirks::q11_setup_error_drops_spd` (also wrong expectation types, CPU percentages outside 0-100, `QueueLength` outside competing-consumer groups). |
| Q12 | composed triggers broken | SS-SPD-2.6 | defined: AND/OR/XOR of the children's results. P1: `Quirks::q12_broken_composed` (also the double evaluation with two measurement stimulus kinds); fixed XOR = exactly one child. |
| Q13 | instantaneous samples merged over containers; aggregation on every event | SS-SPD-2.3/2.4 | proposed (P1): `aggregationOverElements` aggregates the latest value of each element of the group; `getResult` only for events that were aggregated; samples are consumed only when the chain gets that far. P1: `Quirks::q13_merged_samples`; fixed: one window per measuring point, the method over the ready windows, *provisional*. |
| Q14 | deleted elements not reported; population changes ineffective | SS-SPD-5.2 | proposed (F4/P2): deletions are reported, monitors and passive resources of removed elements are dropped, a population change takes effect |
| Q15 | policies react to each other's `SimulationTimeReached` | SS-SPD-2.1 | defined: only the policy's own event. P1: `Quirks::q15_shared_time_events`. |
| Q16 | container order after an adjustment follows Java identity hashes | SS-SPD-4.2 | insertion order (the `det` rule, phase Q2) |
