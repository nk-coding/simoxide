# Semantics decisions

Saved: 2026-11-04T09:04:47.356Z

The same choices, with figures and the reading of each note, are in [semantics-paper.html](./semantics-paper.html){target="_self"}. The machine-readable copy is [semantics-decisions.json](./semantics-decisions.json){target="_self"}. Since 7 November 2026 SimOxide's fixed behaviour (`--spd-quirks fixed`, `--quirks` without Q24) implements these decisions: the cooldown is committed at the request (Q2), an out-of-range step is clamped to the bound by the adjustment instead of being disregarded (Q9), and an Acquire or Release without a demand is a run error (Q24). The paper describes the state before that change. Q1, Q12 and Q26 are open; their fixed behaviour is unchanged ([Quirks](./quirks.md)).

## SPD interpreter

### Q1 — Disregard does not restore adjustor state

Decision: unanswered

### Q2 — Cooldown is committed when the request is made

Decision: Slingshot’s behavior is the right semantics

a cooldown is a constraint on an adjustment made to the target, even though that adjustment may not effectiively change the capacity of the target group

### Q3 — AVERAGE is an exponential moving average

Decision: Keep this behavior

### Q4 — A relative change of 0 ignores the minimum step

Decision: The fixed behavior is the right semantics

### Q5 — Scale-in drops jobs on the removed container

Decision: The fixed behavior is the right semantics

I think also in palladio/slingshot the container drains, the resource container is deleted but the queue in the underlying data structure is cleaned up only after

### Q6 — Every link is connected to every container

Decision: The fixed behavior is the right semantics

### Q7 — Top-down scale-in removes empty containers one enactment late

Decision: The fixed behavior is the right semantics

The fix is the right top-down transformation semantic. It was not deleted to account for requests in transit.
A top-down scale-in assumes, that the effect of scaling in components automatically translates to the scaling in of hosts/containers/vms; however, that does not need to be the case. In practice, these two can be controlled independently.

### Q8 — Top-down placement follows Java hash order

Decision: The fixed behavior is the right semantics

So what is the issue if it changes between JVMs ?

### Q9 — Group size is not checked before the request

Decision: Neither — the rule should be something else

it has to make sure if the change is lt or gt, then the decision is not simply ignored while it lays out of the bounds, but the decision has to be modified to max and min. This was part of the qvto transformation and not in the filter chain in Slingshot.

### Q10 — Static fields in TargetGroupUtils

Decision: The fixed behavior is the right semantics

### Q11 — An unsupported trigger drops the whole SPD

Decision: The fixed behavior is the right semantics

### Q12 — AND / OR / XOR triggers do not combine the children

Decision: Not sure yet

on every event or on relevant events?

### Q13 — One window for every element, consumed on every event

Decision: The fixed behavior is the right semantics

### Q14 — Removed elements stay in the running model

Decision: The fixed behavior is the right semantics

### Q15 — Time policies hear each other’s events

Decision: The fixed behavior is the right semantics

### Q16 — Container order follows Java identity hashes

Decision: The fixed behavior is the right semantics

not sure yet,

## Engine

### Q17 — Several demands of one internal action run together

Decision: The fixed behavior is the right semantics

### Q18 — A throwing handler is disposed and the run continues

Decision: The fixed behavior is the right semantics

### Q19 — A loop count of 0 still enters the body

Decision: The fixed behavior is the right semantics

### Q20 — Roles are resolved by role id alone

Decision: The fixed behavior is the right semantics

### Q21 — A guarded branch with no true guard throws

Decision: The fixed behavior is the right semantics

### Q22 — A collection iterator draws once and never pops the frame

Decision: The fixed behavior is the right semantics

### Q23 — Forks share a cursor and ignore asynchronous children

Decision: The fixed behavior is the right semantics

### Q24 — Acquire and Release require a resource demand

Decision: Slingshot’s behavior is the right semantics

### Q25 — Delay resources abort the user

Decision: The fixed behavior is the right semantics

### Q26 — A stale processor-sharing completion still samples state

Decision: Not sure yet

### Q27 — Passive capacity is evaluated with no frame

Decision: The fixed behavior is the right semantics

### Q28 — Every link transfer draws a failure, even at probability 0

Decision: The fixed behavior is the right semantics

### Q29 — Some response-time monitors record nothing

Decision: The fixed behavior is the right semantics

### Q30 — Entering a composite stops the user

Decision: The fixed behavior is the right semantics

### Q31 — An infrastructure call breaks the assembly-operation monitor

Decision: The fixed behavior is the right semantics

### Q32 — Only simTime stops the run

Decision: The fixed behavior is the right semantics

### Q33 — The entry-call measuring point uses the system role

Decision: The fixed behavior is the right semantics

### Q34 — Replica monitors share one series

Decision: The fixed behavior is the right semantics

### Q35 — Unsupported actions stall with no error

Decision: The fixed behavior is the right semantics

### Q36 — Infrastructure-call inputs are dropped

Decision: The fixed behavior is the right semantics

### Q37 — A call between unconnected containers disposes the link handler

Decision: The fixed behavior is the right semantics
