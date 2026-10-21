#!/usr/bin/env python3
"""Summary of bench/compare/engine.sh: medians over the repeated suite and loading runs, the other
outputs copied as they are. Writes DIR/summary.md.

usage: summarize-engine.py DIR"""
import glob, os, re, statistics as st, sys

D = sys.argv[1]
RAW = os.path.join(D, "raw")
out = []


def table(path):
    """Markdown table rows of a simoxide-bench output: (header, [rows as lists])"""
    hdr, rows = None, []
    for l in open(path, errors="replace"):
        if not l.startswith("|") or l.startswith("|---"):
            continue
        cells = [c.strip() for c in l.strip().strip("|").split("|")]
        if hdr is None:
            hdr = cells
        else:
            rows.append(cells)
    return hdr, rows


def median_tables(pattern):
    """per benchmark and column: the median over the files matching pattern"""
    files = sorted(glob.glob(os.path.join(RAW, pattern)))
    if not files:
        return None, None, 0
    hdr, base = table(files[0])
    vals = {}
    for f in files:
        _, rows = table(f)
        for r in rows:
            for i, c in enumerate(r[1:], 1):
                try:
                    vals.setdefault((r[0], i), []).append(float(c))
                except ValueError:
                    pass
    def fmt(x):
        return str(int(x)) if x == int(x) else f"{x:.4g}"

    rows = [[r[0]] + [fmt(st.median(vals[(r[0], i)])) if (r[0], i) in vals else r[i]
                      for i in range(1, len(r))] for r in base]
    return hdr, rows, len(files)


def md_table(hdr, rows):
    return ["| " + " | ".join(hdr) + " |", "|" + "---|" * len(hdr)] + ["| " + " | ".join(r) + " |" for r in rows]


def col(hdr, name):
    return next(i for i, h in enumerate(hdr) if h.startswith(name))


suites = {}
for k, pat in [("exact", "suite-exact-*.txt"), ("fast", "suite-fast-*.txt"),
               ("slingshot-exact", "suite-slingshot-exact-*.txt"), ("slingshot-fast", "suite-slingshot-fast-*.txt")]:
    hdr, rows, n = median_tables(pat)
    if hdr:
        suites[k] = (hdr, rows)
        out.append(f"### `simoxide-bench suite --reps 3`, mode/flavour {k}: median of {n} invocations\n")
        out += md_table(hdr, rows)
        out.append("")

if "exact" in suites and "fast" in suites:
    he, re_ = suites["exact"]
    hf, rf = suites["fast"]
    ci = col(he, "M requests/s")
    out.append("### Fast vs exact (requests/s, SimuLizar flavour)\n")
    out.append("| benchmark | exact M requests/s | fast M requests/s | fast vs exact |")
    out.append("|---|---|---|---|")
    for a, b in zip(re_, rf):
        x, y = float(a[ci]), float(b[ci])
        out.append(f"| {a[0]} | {x:.3g} | {y:.3g} | {y / x:.2f}x |")
    out.append("")
for m in ("exact", "fast"):
    k = "slingshot-" + m
    if m in suites and k in suites:
        h, a = suites[m]
        _, b = suites[k]
        ci = col(h, "M requests/s")
        rat = []
        out.append(f"### Slingshot flavour vs SimuLizar flavour, {m} RNG mode (requests/s)\n")
        out.append("| benchmark | SimuLizar | Slingshot | ratio |")
        out.append("|---|---|---|---|")
        for x, y in zip(a, b):
            try:
                r = float(y[ci]) / float(x[ci])
            except (ValueError, ZeroDivisionError):
                continue
            rat.append(r)
            out.append(f"| {x[0]} | {x[ci]} | {y[ci]} | {r:.2f} |")
        g = st.geometric_mean(rat) if rat else float("nan")
        out.append(f"\ngeomean {g:.3f}, range {min(rat):.2f} to {max(rat):.2f} ({len(rat)} models)\n")

for f in ("batch-exact.txt", "batch-fast.txt", "fork-exact.txt", "fork-fast.txt", "change-exact.txt", "change-fast.txt"):
    p = os.path.join(RAW, f)
    if os.path.exists(p):
        out.append(f"### {f[:-4]}\n\n```text\n" + open(p).read().strip() + "\n```\n")

for kind in ("load", "load-memory", "load-cache"):
    files = sorted(glob.glob(os.path.join(RAW, f"{kind}-[0-9].txt")))
    v = {}
    for f in files:
        for l in open(f):
            m = re.match(r"(\S+)\s+(\d+)\s+(\d+)\s+([\d.]+)", l)
            if m:
                v.setdefault(m.group(1), []).append(float(m.group(4)))
    if v:
        out.append(f"### {kind} (`load-bench bench --reps 300`, best of 300; median of {len(files)} invocations), µs\n")
        out.append("| model | µs |\n|---|---|")
        out += [f"| {k} | {st.median(x):.1f} |" for k, x in v.items()]
        out.append("")

for kind, rx in (("host-costs", r"core ([\d.]+)ms, compile ([\d.]+)ms, per-run state \(session clone, graph copy\) ([\d.]+)ms \(graph copy ([\d.]+)µs\), fork of the state (\d+)ns, rule run (\d+) µs, change (\d+) µs"),):
    files = sorted(glob.glob(os.path.join(RAW, f"{kind}-*.txt")))
    out.append(f"### {kind} (`tests/host.rs costs`, {len(files)} runs)\n\n```text")
    for f in files:
        out += [l.rstrip() for l in open(f) if "core " in l]
    out.append("```\n")
files = sorted(glob.glob(os.path.join(RAW, "hpa-cost-*.txt")))
out.append(f"### HPA scaling-step cost (`tests/hpa.rs cost_per_scaling_step`, fast mode, {len(files)} runs)\n\n```text")
for f in files:
    out += [l.rstrip() for l in open(f) if l.startswith(("k=", "test cost", "QVTo"))]
out.append("```\n")

open(os.path.join(D, "summary.md"), "w").write("\n".join(out) + "\n")
print(f"wrote {D}/summary.md")
