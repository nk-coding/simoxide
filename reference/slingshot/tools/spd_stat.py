#!/usr/bin/env python3
"""Statistical comparison of two sets of SPD runs (DESIGN-SPD.md V4, phase P3).

    spd_stat.py [--alpha 0.01] [--sim-time T] [--windows K] [--csv OUT] A_DIR... -- B_DIR...

Each DIR holds a run's decision log spd.jsonl (the Slingshot reference runner's, or SimOxide's
--spd-log) and its measurements.csv. Typical use: A = stock Slingshot with seeds 1..30, B =
SimOxide's Slingshot flavour with seeds 1..30.

Per run: the number of executed adjustments, of scale-outs and scale-ins (the first target
group's element count after against before), the time of the first adjustment, the
time-weighted mean and the final element count of the first target group over [0, T], and the
mean response time (series whose metric is a response time at a usage-scenario measuring point)
in K equal windows of [0, T]. Tests between the per-run values: Welch and Mann-Whitney (as
compare_stat.py), and a two-sample KS test on the pooled adjustment times; Holm over all tests.
A statistic without variance on both sides is compared for equality. Pure Python.
"""
import csv
import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from compare_stat import ks, mann_whitney, welch  # noqa: E402


def size_of(summary):
    tg = summary.get("target_groups") or []
    return len(tg[0]["elements"]) if tg else 0


def run_stats(d, T, K):
    adj = []
    with open(os.path.join(d, "spd.jsonl")) as fh:
        for line in fh:
            v = json.loads(line)
            if v.get("k") == "adjust":
                adj.append(v)
    ok = [a for a in adj if a.get("ok")]
    outs = ins = 0
    times = []
    traj = []  # (t, size)
    size0 = None
    for a in ok:
        b, f = size_of(a.get("before", {})), size_of(a.get("after", {}))
        if size0 is None:
            size0 = b
        outs += f > b
        ins += f < b
        times.append(a["t"])
        traj.append((a["t"], f))
    if size0 is None:
        size0 = 1
    acc, t0, s = 0.0, 0.0, size0
    for t, f in traj:
        acc += s * (min(t, T) - t0)
        t0, s = min(t, T), f
    acc += s * (T - t0)
    st = {
        "adjustments": len(ok),
        "scale_outs": outs,
        "scale_ins": ins,
        "first_adjustment": times[0] if times else T,
        "mean_size": acc / T,
        "final_size": s,
    }
    rt = [[] for _ in range(K)]
    with open(os.path.join(d, "measurements.csv"), newline="") as fh:
        r = csv.reader(fh)
        next(r, None)
        for mp, metric, t, v in r:
            if mp.startswith("UsageScenarioMeasuringPoint") and "Response Time" in metric:
                k = min(K - 1, int(float(t) / T * K))
                rt[k].append(float(v))
    for k in range(K):
        if rt[k]:
            st[f"rt_mean_w{k}"] = sum(rt[k]) / len(rt[k])
    return st, times


def main(argv):
    alpha, T, K, out = 0.01, 600.0, 4, None
    args = []
    it = iter(argv)
    for x in it:
        if x == "--alpha":
            alpha = float(next(it))
        elif x == "--sim-time":
            T = float(next(it))
        elif x == "--windows":
            K = int(next(it))
        elif x == "--csv":
            out = next(it)
        else:
            args.append(x)
    if "--" not in args:
        sys.exit(__doc__)
    k = args.index("--")
    A = [run_stats(d, T, K) for d in args[:k]]
    B = [run_stats(d, T, K) for d in args[k + 1:]]
    keys = sorted(set().union(*[set(s) for s, _ in A + B]))
    tests = []
    desc = lambda x: f"{sum(x)/len(x):.6g} [{min(x):.6g}, {max(x):.6g}]"
    for key in keys:
        xa = [s[key] for s, _ in A if key in s]
        xb = [s[key] for s, _ in B if key in s]
        if not xa or not xb:
            continue
        if min(xa) == max(xa) and min(xb) == max(xb):
            tests.append((key, "equal", 1.0 if xa[0] == xb[0] else 0.0, desc(xa), desc(xb)))
            continue
        tests.append((key, "welch", welch(xa, xb), desc(xa), desc(xb)))
        tests.append((key, "mann-whitney", mann_whitney(xa, xb), desc(xa), desc(xb)))
    ta = [t for _, ts in A for t in ts]
    tb = [t for _, ts in B for t in ts]
    if ta and tb:
        tests.append(("adjustment_times", "ks", ks(ta, tb), f"n={len(ta)}", f"n={len(tb)}"))
    pv = sorted((t[2], i) for i, t in enumerate(tests) if t[1] != "equal" and not math.isnan(t[2]))
    m = len(pv)
    reject = set()
    for rank, (p, i) in enumerate(pv):
        if p <= alpha / (m - rank):
            reject.add(i)
        else:
            break
    print(f"A: {len(A)} runs, B: {len(B)} runs, {len(tests)} tests, Holm alpha {alpha}")
    print("| statistic | test | p | A mean [min, max] | B mean [min, max] | result |")
    print("|---|---|---|---|---|---|")
    fails = 0
    rows = []
    for i, (key, test, p, da, db) in enumerate(tests):
        bad = (test == "equal" and p == 0.0) or i in reject
        fails += bad
        res = "DIFFERENT" if bad else "ok"
        print(f"| {key} | {test} | {p:.3g} | {da} | {db} | {res} |")
        rows.append([key, test, p, da, db, res])
    if out:
        with open(out, "w", newline="") as fh:
            w = csv.writer(fh)
            w.writerow(["statistic", "test", "p", "a", "b", "result"])
            w.writerows(rows)
    print(f"{fails} of {len(tests)} tests reject equality")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
