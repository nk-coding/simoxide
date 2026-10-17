#!/usr/bin/env python3
"""Statistical comparison of two sets of runs (measurements.csv in SimOxide's format), per series.

    compare_stat.py [--alpha 0.01] [--csv OUT] A_DIR... -- B_DIR...

Each DIR is a run directory with measurements.csv (or the CSV file itself). Typical use: A = the
Slingshot reference runner in stock mode with seeds 1..30, B = SimOxide's Slingshot flavour with
seeds 1..30 (docs/correctness/slingshot.md, DESIGN-SPD.md S2 / V4).

Per series (measuring point, metric) and run: the number of tuples, the mean, the time-weighted
mean (state-like series: the value holds until the next tuple; first to last tuple), p50 and p90.
Tests between the two sets of per-run values: Welch's t-test and the Mann-Whitney U test (normal
approximation, ties corrected) on the per-run means, and a two-sample Kolmogorov-Smirnov test on
the pooled values (asymptotic distribution). Holm's correction over all tests of all series. A
set without variance in a statistic (deterministic runs) is compared by whether the other set's
range contains its value ("range"). Pure Python, no dependencies.
"""
import csv
import math
import os
import sys


def load(path):
    f = os.path.join(path, "measurements.csv") if os.path.isdir(path) else path
    series = {}
    with open(f, newline="") as fh:
        r = csv.reader(fh)
        next(r, None)
        for mp, metric, t, v in r:
            series.setdefault((mp, metric), []).append((float(t), float(v)))
    return series


def summary(rows):
    vals = [v for _, v in rows]
    n = len(vals)
    if n == 0:
        return {"count": 0}
    s = sorted(vals)
    tw = float("nan")
    if n > 1 and rows[-1][0] > rows[0][0]:
        acc = 0.0
        for (t0, v0), (t1, _) in zip(rows, rows[1:]):
            acc += v0 * (t1 - t0)
        tw = acc / (rows[-1][0] - rows[0][0])
    return {
        "count": n,
        "mean": sum(vals) / n,
        "twmean": tw,
        "p50": s[min(n - 1, int(0.5 * n))],
        "p90": s[min(n - 1, int(0.9 * n))],
    }


def norm_sf2(z):
    """Two-sided p-value of a standard normal statistic."""
    return math.erfc(abs(z) / math.sqrt(2))


def welch(a, b):
    na, nb = len(a), len(b)
    if na < 2 or nb < 2:
        return float("nan")
    ma, mb = sum(a) / na, sum(b) / nb
    va = sum((x - ma) ** 2 for x in a) / (na - 1)
    vb = sum((x - mb) ** 2 for x in b) / (nb - 1)
    se = math.sqrt(va / na + vb / nb)
    if se == 0:
        return 1.0 if ma == mb else 0.0
    t = (ma - mb) / se
    # df for the record; with 30 runs per side the normal approximation of t is close enough
    return norm_sf2(t)


def mann_whitney(a, b):
    na, nb = len(a), len(b)
    if na == 0 or nb == 0:
        return float("nan")
    allv = sorted([(x, 0) for x in a] + [(x, 1) for x in b])
    ranks = [0.0] * len(allv)
    ties = 0.0
    i = 0
    while i < len(allv):
        j = i
        while j + 1 < len(allv) and allv[j + 1][0] == allv[i][0]:
            j += 1
        r = (i + j) / 2 + 1
        for k in range(i, j + 1):
            ranks[k] = r
        t = j - i + 1
        ties += t ** 3 - t
        i = j + 1
    ra = sum(r for r, (_, g) in zip(ranks, allv) if g == 0)
    u = ra - na * (na + 1) / 2
    n = na + nb
    var = na * nb / 12 * ((n + 1) - ties / (n * (n - 1)))
    if var <= 0:
        return 1.0
    z = (u - na * nb / 2) / math.sqrt(var)
    return norm_sf2(z)


def ks(a, b):
    na, nb = len(a), len(b)
    if na == 0 or nb == 0:
        return float("nan")
    a, b = sorted(a), sorted(b)
    i = j = 0
    d = 0.0
    while i < na and j < nb:
        x = min(a[i], b[j])
        while i < na and a[i] <= x:
            i += 1
        while j < nb and b[j] <= x:
            j += 1
        d = max(d, abs(i / na - j / nb))
    en = math.sqrt(na * nb / (na + nb))
    lam = (en + 0.12 + 0.11 / en) * d
    if lam < 1e-3:
        return 1.0
    p = 0.0
    for k in range(1, 101):
        p += 2 * (-1) ** (k - 1) * math.exp(-2 * k * k * lam * lam)
    return min(1.0, max(0.0, p))


def main(argv):
    alpha, out = 0.01, None
    args = []
    it = iter(argv)
    for x in it:
        if x == "--alpha":
            alpha = float(next(it))
        elif x == "--csv":
            out = next(it)
        else:
            args.append(x)
    if "--" not in args:
        sys.exit(__doc__)
    k = args.index("--")
    A = [load(p) for p in args[:k]]
    B = [load(p) for p in args[k + 1 :]]
    keys = sorted(set().union(*[set(r) for r in A + B]))
    tests = []  # (key, stat, test, p, a_desc, b_desc)
    for key in keys:
        sa = [summary(r.get(key, [])) for r in A]
        sb = [summary(r.get(key, [])) for r in B]
        for stat in ("count", "mean", "twmean"):
            xa = [s[stat] for s in sa if stat in s and not math.isnan(s[stat])]
            xb = [s[stat] for s in sb if stat in s and not math.isnan(s[stat])]
            if not xa or not xb:
                continue
            desc = lambda x: f"{sum(x)/len(x):.6g} [{min(x):.6g}, {max(x):.6g}]"
            if min(xb) == max(xb) or min(xa) == max(xa):
                # a deterministic side: inside the other side's range?
                inside = (min(xa) <= sum(xb) / len(xb) <= max(xa)) if min(xb) == max(xb) else (
                    min(xb) <= sum(xa) / len(xa) <= max(xb))
                tests.append((key, stat, "range", 1.0 if inside else 0.0, desc(xa), desc(xb)))
                continue
            tests.append((key, stat, "welch", welch(xa, xb), desc(xa), desc(xb)))
            tests.append((key, stat, "mann-whitney", mann_whitney(xa, xb), desc(xa), desc(xb)))
        pa = [v for r in A for _, v in r.get(key, [])]
        pb = [v for r in B for _, v in r.get(key, [])]
        if pa and pb and len(set(pa + pb)) > 1:
            tests.append((key, "values", "ks", ks(pa, pb), f"n={len(pa)}", f"n={len(pb)}"))
    # Holm over the tests with p-values (range checks are pass/fail)
    pv = sorted((t[3], i) for i, t in enumerate(tests) if t[2] != "range" and not math.isnan(t[3]))
    m = len(pv)
    reject = set()
    for rank, (p, i) in enumerate(pv):
        if p <= alpha / (m - rank):
            reject.add(i)
        else:
            break
    fails = 0
    print(f"A: {len(A)} runs, B: {len(B)} runs, {len(tests)} tests, Holm alpha {alpha}")
    print("| series | statistic | test | p | A mean [min, max] | B mean [min, max] | result |")
    print("|---|---|---|---|---|---|---|")
    rows = []
    for i, (key, stat, test, p, da, db) in enumerate(tests):
        bad = (test == "range" and p == 0.0) or i in reject
        fails += bad
        res = "DIFFERENT" if bad else "ok"
        print(f"| {key[0]} {key[1]} | {stat} | {test} | {p:.3g} | {da} | {db} | {res} |")
        rows.append([key[0], key[1], stat, test, p, da, db, res])
    if out:
        with open(out, "w", newline="") as fh:
            w = csv.writer(fh)
            w.writerow(["measuring_point", "metric", "statistic", "test", "p", "a", "b", "result"])
            w.writerows(rows)
    print(f"{fails} of {len(tests)} tests reject equality")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
