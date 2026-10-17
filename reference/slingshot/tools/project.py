#!/usr/bin/env python3
"""Event projection of a Slingshot reference run for the Slingshot-exact comparison
(docs/spec/slingshot/exact.md, rules SS-X-*).

    tools/project.py RUN_DIR [OUT_DIR]      reads RUN_DIR/trace.jsonl, tape.jsonl; writes to OUT_DIR
                                            (default RUN_DIR): projection.jsonl, projection-m.jsonl,
                                            tape-projection.jsonl

- projection.jsonl (level E): the semantic events of the bus trace (users, usage and SEFF passes,
  calls, resource requests and jobs, passive grants, measurements, aborts, SPD, stop), renamed,
  with users and jobs renumbered by first appearance in the projection.
- projection-m.jsonl (level M): only measurements, aborts, SPD events and the stop.
- tape-projection.jsonl: every draw with its stream, per-stream index, value, site purpose `o`
  (SS-X-4) and `p`, the index of the last level-E line at or before the dispatched event of the
  draw (-1: before the first one).

Both projections are deterministic functions of the det trace; SimOxide's Slingshot-exact mode
writes the same three files itself. Format: one JSON object per line, keys in the order given in
SS-X-2, numbers as in the trace (Java Double.toString).
"""
import json
import os
import sys
from collections import OrderedDict

# bus event -> (projected name, keys to keep, level) ; level "m" lines are in both projections
RULES = OrderedDict([
    ("UsageModelPassedElement", ("pass", ["u", "type", "el"], "e")),
    ("SEFFModelPassedElement", ("pass", ["u", "ac", "type", "el"], "e")),
    ("UserEntryRequested", ("call", ["u", "role", "sig"], "e")),
    ("UserRequestFinished", ("call_end", ["u", "role", "sig"], "e")),
    ("SEFFExternalActionCalled", ("ext", ["u", "ac", "role", "sig"], "e")),
    ("SEFFInfrastructureCalled", ("infra", ["u", "ac", "role", "sig"], "e")),
    ("ResourceDemandRequested", ("request", ["u", "ac", "kind", "res", "action", "passive"], "e")),
    ("JobInitiated", ("job", ["job", "u", "rc", "res", "link", "d"], "e")),
    ("JobFinished", ("job_end", ["job"], "e")),
    ("JobAborted", ("job_abort", ["job"], "m")),
    ("PassiveResourceAcquired", ("grant", ["u", "ac", "passive"], "e")),
    ("PassiveResourceReleased", ("release", ["u", "ac", "passive"], "e")),
    ("UserFinished", ("user_end", ["u"], "e")),
    ("UserAborted", ("user_abort", ["u"], "m")),
    ("MeasurementMade", ("meas", ["mp", "metric", "v"], "m")),
    ("SimulationTimeReached", ("spd_time", ["tg"], "m")),
    ("ModelAdjustmentRequested", ("spd_request", ["policy"], "m")),
    ("ModelAdjusted", ("spd_adjusted", ["ok", "changes", "new", "deleted"], "m")),
    ("SimulationFinished", ("stop", [], "m")),
])

# StoEx draw site (innermost Slingshot frame, tape "o") -> SimOxide origin purpose (SS-X-4)
STOEX_SITES = {
    "UsageScenarioInterpreter.caseStart": "interarrival",
    "ClosedWorkloadUserInitiated.<init>": "think",
    "UsageScenarioInterpreter.caseDelay": "delay",
    "UsageScenarioInterpreter.caseLoop": "loop",
    "SeffInterpreter.caseLoopAction": "loop",
    "SeffInterpreter.caseCollectionIteratorAction": "collection",
    "SeffInterpreter.caseSetVariableAction": "setvar",
    "TransitionDeterminer.conditionHolds": "guard",
    "InfrastructureCallsContextHolder.<init>": "infra",
    "ResourceSimulation.initiateActiveResource": "demand",
    "ResourceSimulation.createWaitingJob": "passive",
    "PassiveResourceTable.createPassiveResources": "capacity",
    "AbstractActiveResource.onJobInitiated": "rate",
    "SimulatedLinkingResource.process": "latency",
    "SystemSimulationBehavior.onUserEntryRequested": "param",
    "SystemSimulationBehavior.requestCallOverWire": "param",
    "RepositoryInterpreter.caseBasicComponent": "component",
    "UsageSimulationBehavior.onUserRequestFinished": "return",
    "SystemSimulationBehavior.onCallOverWireSucceeded": "return",
}


def purpose(s, o):
    if s == "math":
        return o or "?"
    return STOEX_SITES.get(o, "?")


class Raw(str):
    """A JSON number kept as its source text, so that Java's Double.toString format survives."""


def loads(line):
    return json.loads(line, object_pairs_hook=OrderedDict, parse_float=Raw, parse_int=Raw)


def dumps(v):
    if isinstance(v, Raw):
        return str(v)
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, list):
        return "[" + ",".join(dumps(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{" + ",".join(json.dumps(k) + ":" + dumps(x) for k, x in v.items()) + "}"
    if v is None:
        return "null"
    raise TypeError(type(v))


def project(run, out):
    trace = os.path.join(run, "trace.jsonl")
    tape = os.path.join(run, "tape.jsonl")
    users, jobs = {}, {}
    e_lines, m_lines = [], []
    last_e = {}  # trace event index -> index of the last level-E line at or before it
    idx = -1
    with open(trace) as f:
        for line in f:
            d = loads(line)
            ev = d["ev"]
            if ev in ("header", "finish"):
                continue
            idx += 1
            r = RULES.get(ev)
            if r is not None:
                name, keys, level = r
                o = OrderedDict([("ev", name), ("t", d["t"])])
                for k in keys:
                    if k not in d:
                        continue
                    v = d[k]
                    if k == "u":
                        v = users.setdefault(v, len(users) + 1)
                    elif k == "job":
                        v = jobs.setdefault(v, len(jobs) + 1)
                    o[k] = v
                e_lines.append(o)
                if level == "m":
                    m_lines.append(o)
            last_e[idx] = len(e_lines) - 1
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "projection.jsonl"), "w") as f:
        for o in e_lines:
            f.write(dumps(o) + "\n")
    with open(os.path.join(out, "projection-m.jsonl"), "w") as f:
        for o in m_lines:
            f.write(dumps(o) + "\n")
    n = 0
    if os.path.exists(tape):
        with open(tape) as f, open(os.path.join(out, "tape-projection.jsonl"), "w") as w:
            for line in f:
                d = loads(line)
                p = last_e.get(int(d["e"]), -1)
                o = OrderedDict([("s", d["s"]), ("i", d["i"]), ("u", d["u"]),
                                 ("o", purpose(d["s"], d.get("o"))), ("p", p)])
                w.write(dumps(o) + "\n")
                n += 1
    return len(e_lines), len(m_lines), n


def main():
    run = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else run
    e, m, n = project(run, out)
    print(f"{out}: projection {e} lines, level M {m} lines, tape {n} draws")


if __name__ == "__main__":
    main()
