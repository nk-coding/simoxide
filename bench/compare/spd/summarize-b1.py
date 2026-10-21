#!/usr/bin/env python3
"""Summary of benchmark B1 (bench/compare/spd/b1.sh): reads RAW (the raw logs) and WORK/ss (Slingshot's
timings.csv), writes OUT/summary.md and OUT/b1.csv.

usage: summarize-b1.py RAW OUT"""
import csv, glob, json, os, re, statistics as st, sys

RAW, OUT = sys.argv[1], sys.argv[2]
WORK = os.path.dirname(RAW.rstrip("/"))
SS = os.path.join(WORK, "ss")
rows = []  # (kind, sim, model, len, rep, key, value)


def kv(line):
    return dict(m.groups() for m in re.finditer(r"(\w+)=(\S+)", line))


def time_line(path):
    for l in open(path, errors="replace"):
        if l.startswith("TIME "):
            return kv(l)
    return None


def med(v):
    return st.median(v) if v else float("nan")


def spread(v):
    """p10-p90 (or min-max for fewer than 10 values)"""
    if not v:
        return (float("nan"), float("nan"))
    s = sorted(v)
    if len(s) < 10:
        return (s[0], s[-1])
    q = st.quantiles(s, n=10)
    return (q[0], q[-1])


def fmt_ms(x):
    if x != x:
        return "–"
    if x >= 1000:
        return f"{x / 1000:.3g} s"
    if x >= 1:
        return f"{x:.3g} ms"
    return f"{x * 1000:.3g} µs"


def ratio(a, b):
    return f"{a / b:,.0f}×".replace(",", " ") if a == a and b == b and b > 0 else "–"


def parse_name(p):
    b = os.path.basename(p)[:-4]
    return b.split("__")


def ss_warm(tm):
    """Slingshot's warm runs: the JVM's runs after a warm-up of a third of them (at least run 0)"""
    return [float(r["wall_ms"]) for r in tm[max(1, len(tm) // 3):]]


def ss_cold(log, tm):
    """Slingshot one-shot: JVM (Equinox) start-up until the runner starts, plus its first run"""
    for l in open(log, errors="replace"):
        m = re.search(r"JVM uptime at start (\d+) ms", l)
        if m and tm:
            return int(m.group(1)) + float(tm[0]["wall_ms"])
    return None


def ss_timings(d):
    f = os.path.join(d, "timings.csv")
    if not os.path.exists(f):
        return None
    return list(csv.DictReader(open(f)))


def usage_rt(csvfile):
    n, s = 0, 0.0
    if not os.path.exists(csvfile):
        return None
    for r in csv.reader(open(csvfile)):
        if r and r[0].startswith("UsageScenarioMeasuringPoint") and r[1].startswith("Response Time"):
            n += 1
            s += float(r[3])
    return (n, s / n if n else float("nan"))


# ---------------------------------------------------------------- plain
plain = {}  # (sim, model, len) -> dict(warm=[], cold=[], rss=[], cold_rss=[], req=..., rt=..., simend=...)


def P(sim, m, ln):
    return plain.setdefault((sim, m, ln), {"warm": [], "cold": [], "rss": [], "cold_rss": [], "first": [],
                                           "res": set(), "simend": []})


for f in glob.glob(os.path.join(RAW, "plain__*.log")):
    kind, sim, m, ln, rep = parse_name(f)
    d = P(sim, m, ln)
    t = time_line(f)
    if t and t.get("rc") == "0":
        d["rss"].append(int(t["rss_kb"]))
    if sim == "slingshot":
        tm = ss_timings(os.path.join(SS, f"plain-{m}-{ln}-{rep}"))
        if tm:
            d["first"].append(float(tm[0]["wall_ms"]))
            d["warm"] += ss_warm(tm)
            d["simend"].append(float(tm[0]["sim_time"]))
            d["events"] = int(tm[0]["events"])
        c = ss_cold(f, tm)
        if c:
            d["cold"].append(c)
    else:
        for l in open(f, errors="replace"):
            if l.startswith("RESULT") and "phase=run" in l:
                r = kv(l)
                d["warm"].append(float(r["wall_ms"]))
                d.setdefault("sim", []).append(float(r["sim_ms"]))
                d["res"].add((rep, r["requests"], r["mean_rt"]))
                d["simend"].append(float(r["sim_end"]))
                d["events"] = int(r["events"])
for f in glob.glob(os.path.join(RAW, "cold__*.log")):
    kind, sim, m, ln, rep = parse_name(f)
    t = time_line(f)
    if t and t.get("rc") == "0":
        d = P(sim, m, ln)
        d["cold"].append(float(t["wall_s"]) * 1000)
        d["cold_rss"].append(int(t["rss_kb"]))

SIMS_PLAIN = ["simoxide-slingshot-fast", "simoxide-slingshot-exact-exact", "simoxide-simulizar-exact",
              "simoxide-simulizar-fast", "slingshot"]
NAMES = {"simoxide-slingshot-fast": "SimOxide Slingshot flavour (fast)",
         "simoxide-slingshot-exact-exact": "SimOxide Slingshot-exact",
         "simoxide-simulizar-exact": "SimOxide SimuLizar flavour (exact)",
         "simoxide-simulizar-fast": "SimOxide SimuLizar flavour (fast)",
         "simoxide-simulizar-fast-snap": "",
         "slingshot": "Slingshot (stock nightly, S0 runner)"}
md = []
md.append("### Plain models: SimOxide's flavours against Slingshot\n")
md.append("Warm per-run wall time, median over all measured runs of the processes (p10–p90 in brackets); "
          "SimOxide: `simoxide-compare` (load, compile, run, summaries per run); Slingshot: the S0 runner "
          "(`--stock`, seeded, no instrumentation or measurement output), each JVM's runs after a warm-up of a third of them (31 short, 8 long runs per JVM). "
          "Speed-up = Slingshot / SimOxide.\n")
models = sorted({(m, ln) for (_, m, ln) in plain}, key=lambda x: (x[0], x[1] != "short"))
hdr = "| model | len | sim. s | " + " | ".join(NAMES[s] for s in SIMS_PLAIN) + " | Slingshot / Slingshot flavour | Slingshot / Slingshot-exact |"
md.append(hdr)
md.append("|" + "---|" * (hdr.count("|") - 1))
for m, ln in models:
    cells = []
    meds = {}
    for s in SIMS_PLAIN:
        d = plain.get((s, m, ln))
        if not d or not d["warm"]:
            cells.append("–")
            continue
        meds[s] = med(d["warm"])
        lo, hi = spread(d["warm"])
        cells.append(f"{fmt_ms(meds[s])} ({fmt_ms(lo)}–{fmt_ms(hi)})")
        rows.append(("plain-warm", s, m, ln, "", "median_ms", meds[s]))
    simend = (plain.get(("simoxide-slingshot-fast", m, ln)) or {}).get("simend") or [float("nan")]
    md.append(f"| `{m}` | {ln} | {simend[0]:g} | " + " | ".join(cells) +
              f" | {ratio(meds.get('slingshot', float('nan')), meds.get('simoxide-slingshot-fast', float('nan')))}"
              f" | {ratio(meds.get('slingshot', float('nan')), meds.get('simoxide-slingshot-exact-exact', float('nan')))} |")

md.append("\n**Cold one-shot** (process start to results, median of the processes; SimOxide: `simoxide run` "
          "with `--measurements`; Slingshot: the JVM's uptime when the S0 runner starts, i.e. JVM and Equinox start-up, plus its first run) and "
          "**first run in the JVM** (Slingshot, excluding JVM and Equinox start-up), peak RSS of the warm processes:\n")
hdr = "| model | len | " + " | ".join(NAMES[s] for s in SIMS_PLAIN) + " | Slingshot first run in JVM | peak RSS SimOxide (Slingshot flavour) / Slingshot |"
md.append(hdr)
md.append("|" + "---|" * (hdr.count("|") - 1))
for m, ln in models:
    cells = []
    for s in SIMS_PLAIN:
        d = plain.get((s, m, ln))
        cells.append(fmt_ms(med(d["cold"])) if d and d["cold"] else "–")
        if d and d["cold"]:
            rows.append(("plain-cold", s, m, ln, "", "median_ms", med(d["cold"])))
    ss = plain.get(("slingshot", m, ln), {})
    so = plain.get(("simoxide-slingshot-fast", m, ln), {})
    rss = f"{max(so.get('rss') or [0]) / 1024:.0f} MB / {max(ss.get('rss') or [0]) / 1024:.0f} MB"
    md.append(f"| `{m}` | {ln} | " + " | ".join(cells) + f" | {fmt_ms(med(ss.get('first', [])))} | {rss} |")

md.append("\n**Simulation only** (SimOxide: the run without loading and compiling, median `sim_ms` of the warm runs) "
          "against Slingshot's warm run (which includes its model loading, about 10 % of it):\n")
md.append("| model | len | " + " | ".join(NAMES[s] for s in SIMS_PLAIN[:4]) + " |")
md.append("|" + "---|" * 6)
for m, ln in models:
    cells = [fmt_ms(med(plain.get((s, m, ln), {}).get("sim", []))) for s in SIMS_PLAIN[:4]]
    for s_, c in zip(SIMS_PLAIN[:4], cells):
        rows.append(("plain-sim-only", s_, m, ln, "", "median_ms", med(plain.get((s_, m, ln), {}).get("sim", []))))
    md.append(f"| `{m}` | {ln} | " + " | ".join(cells) + " |")

# sanity
md.append("\n**Results** (seed 1; usage-scenario requests and mean response time): SimOxide's runs per flavour and mode, and one Slingshot run with measurements (stock Slingshot draws branch decisions from `Math.random()`, so it is not reproducible per seed):\n")
md.append("| model | len | " + " | ".join(NAMES[s] for s in SIMS_PLAIN) + " |")
md.append("|" + "---|" * (len(SIMS_PLAIN) + 2))
for m, ln in models:
    cells = []
    for s in SIMS_PLAIN:
        if s == "slingshot":
            r = usage_rt(os.path.join(SS, f"check-{m}-{ln}", "measurements.csv"))
            cells.append(f"{r[0]} / {r[1]:.6g}" if r else "–")
            continue
        d = plain.get((s, m, ln))
        res = sorted(x for x in (d or {}).get("res", set()) if x[0] == "1")
        vals = sorted({(a, b) for _, a, b in res})
        cells.append("; ".join(f"{a} / {float(b):.6g}" for a, b in vals) if vals else "–")
    md.append(f"| `{m}` | {ln} | " + " | ".join(cells) + " |")

# ---------------------------------------------------------------- SPD
spd = {}
for f in glob.glob(os.path.join(RAW, "spd__*.log")):
    kind, sim, m, rep = parse_name(f)
    d = spd.setdefault((sim, m), {"warm": [], "adj": [], "n": [], "first": [], "rss": [], "simend": [],
                                  "setup": [], "events": [], "req": [], "cold": []})
    t = time_line(f)
    if t and t.get("rc") == "0":
        d["rss"].append(int(t["rss_kb"]))
    if sim == "slingshot":
        tm = ss_timings(os.path.join(SS, f"spd-{m}-{rep}"))
        if tm:
            d["first"].append(float(tm[0]["wall_ms"]))
            d["warm"] += ss_warm(tm)
            d["simend"].append(float(tm[0]["sim_time"]))
            d["events"].append(int(tm[0]["events"]))
        c = ss_cold(f, tm)
        if c:
            d["cold"].append(c)
    else:
        for l in open(f, errors="replace"):
            if l.startswith("SETUP"):
                d["setup"].append(float(kv(l)["qvto_setup_ms"]))
            if l.startswith("RUN") and "phase=run" in l:
                r = kv(l)
                d["warm"].append(float(r["wall_ms"]))
                d["adj"].append(float(r["adjust_ms"]))
                d["n"].append(int(r["adjustments"]))
                d["simend"].append(float(r["sim_end"]))
                d["events"].append(int(r["events"]))
                d["req"].append(int(r["requests"]))
for f in glob.glob(os.path.join(RAW, "spdcold__*.log")):
    kind, sim, m, rep = parse_name(f)
    t = time_line(f)
    if t and t.get("rc") == "0":
        spd.setdefault((sim + "-cold", m), {"cold": []})["cold"].append(float(t["wall_s"]) * 1000)

SPD_SIMS = ["simoxide-slingshot-fast", "simoxide-simulizar-fast", "simoxide-simulizar-exact", "slingshot"]
SPDQ = "simoxide-slingshotq-fast"
spd_models = sorted({m for (_, m) in spd}, key=lambda m: (m.endswith("_long"), m))
md.append("\n### SPD workloads\n")
md.append("Warm per-run wall time (median; p10–p90), SimOxide: `spd-bench` (simulation created, run with its "
          "scaling policies and QVTo adjustments, result; the QVTo core and `MainTransformation` are set up once "
          "per process), Slingshot: S0 runner, each JVM's runs after a warm-up of a third of them. adj = adjustments per run (median), "
          "in adj. = share of the run's wall time spent in adjustments (QVTo plus the change path).\n")
hdr = ("| model | sim. s | Slingshot flavour (fast) | adj | in adj. | SimuLizar flavour (fast) | adj | in adj. | "
       "SimuLizar flavour (exact) | Slingshot | Slingshot / Slingshot flavour | sim. s/s Slingshot flavour | sim. s/s Slingshot |")
md.append(hdr)
md.append("|" + "---|" * (hdr.count("|") - 1))
for m in spd_models:
    cells = []
    meds = {}
    for s in SPD_SIMS:
        d = spd.get((s, m))
        if not d or not d["warm"]:
            cells.append("–")
            if s.startswith("simoxide-s") and s != "simoxide-simulizar-exact":
                cells += ["–", "–"]
            continue
        meds[s] = med(d["warm"])
        lo, hi = spread(d["warm"])
        cells.append(f"{fmt_ms(meds[s])} ({fmt_ms(lo)}–{fmt_ms(hi)})")
        rows.append(("spd-warm", s, m, "", "", "median_ms", meds[s]))
        if s in ("simoxide-slingshot-fast", "simoxide-simulizar-fast"):
            share = sum(d["adj"]) / sum(d["warm"]) if sum(d["warm"]) else float("nan")
            cells += [f"{med(d['n']):g}", f"{share * 100:.0f} %"]
            rows.append(("spd-adjust-share", s, m, "", "", "share", share))
            rows.append(("spd-adjust-ms-per", s, m, "", "", "ms", sum(d["adj"]) / max(1, sum(d["n"]))))
    se = spd.get(("simoxide-slingshot-fast", m), {}).get("simend") or [float("nan")]
    sss = spd.get(("slingshot", m), {}).get("simend") or [float("nan")]
    sps_so = se[0] / (meds.get("simoxide-slingshot-fast", float("nan")) / 1000)
    sps_ss = sss[0] / (meds.get("slingshot", float("nan")) / 1000)
    md.append(f"| `{m}` | {se[0]:g} | " + " | ".join(cells) +
              f" | {ratio(meds.get('slingshot', float('nan')), meds.get('simoxide-slingshot-fast', float('nan')))}"
              f" | {sps_so:,.0f} | {sps_ss:,.0f} |".replace(",", " "))

md.append("\nPer adjustment (SimOxide, QVTo `MainTransformation` plus the change path, mean over all measured runs), "
          "QVTo set-up per process, cold one-shot `simoxide run` (incl. QVTo set-up), Slingshot cold "
          "(JVM with Equinox, first run) and peak RSS:\n")
hdr = ("| model | ms per adjustment, Slingshot flavour | ms per adjustment, SimuLizar flavour | QVTo set-up ms | "
       "cold `simoxide run` Slingshot flavour / Slingshot-exact / SimuLizar exact | Slingshot cold process | "
       "Slingshot first run in JVM | peak RSS SimOxide / Slingshot |")
md.append(hdr)
md.append("|" + "---|" * (hdr.count("|") - 1))
for m in spd_models:
    a = spd.get(("simoxide-slingshot-fast", m), {})
    b = spd.get(("simoxide-simulizar-fast", m), {})
    ss = spd.get(("slingshot", m), {})
    per = lambda d: sum(d.get("adj", [])) / max(1, sum(d.get("n", []))) if d.get("n") else float("nan")
    colds = " / ".join(fmt_ms(med(spd.get((s + "-cold", m), {}).get("cold", []))) for s in
                       ("simoxide-slingshot-fast", "simoxide-slingshot-exact-exact", "simoxide-simulizar-exact"))
    rss = f"{max(a.get('rss') or [0]) / 1024:.0f} MB / {max(ss.get('rss') or [0]) / 1024:.0f} MB"
    md.append(f"| `{m}` | {per(a):.3f} | {per(b):.3f} | {med(a.get('setup', [])):.1f} | {colds} | "
              f"{fmt_ms(med(ss.get('cold', [])))} | {fmt_ms(med(ss.get('first', [])))} | {rss} |")

if any(k[0] == SPDQ for k in spd):
    md.append("\n**With stock Slingshot's quirks** (Slingshot flavour, fast, `quirks=fast spdQuirks=slingshot` as in "
              "`reference/slingshot/spd-stat.sh`: the decisions stock Slingshot makes, Slingshot itself scales "
              "`spd_v4_cpu` about 16 times in 600 s):\n")
    md.append("| model | warm per run | adjustments (min–max) | in adj. | ms per adjustment | requests (min–max) | Slingshot / this |")
    md.append("|---|---|---|---|---|---|---|")
    for m in spd_models:
        d = spd.get((SPDQ, m))
        if not d or not d["warm"]:
            continue
        w = med(d["warm"]); ss = med(spd.get(("slingshot", m), {}).get("warm", []))
        share = sum(d["adj"]) / sum(d["warm"])
        per = sum(d["adj"]) / max(1, sum(d["n"]))
        rows.append(("spdq-warm", SPDQ, m, "", "", "median_ms", w))
        md.append(f"| `{m}` | {fmt_ms(w)} | {min(d['n'])}–{max(d['n'])} | {share * 100:.0f} % | {per:.3f} | "
                  f"{min(d['req'])}–{max(d['req'])} | {ratio(ss, w)} |")

# sanity: adjustments and requests
md.append("\n**Results** (seed 1): SimOxide's adjustments and finished requests per run (all seeds: min–max), and Slingshot's "
          "adjustments in its SPD log (one instrumented run, unseedable branch decisions):\n")
md.append("| model | Slingshot flavour: adjustments, requests | SimuLizar flavour (fast) | Slingshot: adjustments (SPD log), requests |")
md.append("|---|---|---|---|")
for m in spd_models:
    def rng(d, k):
        v = d.get(k, [])
        return f"{min(v)}–{max(v)}" if v else "–"
    a = spd.get(("simoxide-slingshot-fast", m), {})
    b = spd.get(("simoxide-simulizar-fast", m), {})
    chk = os.path.join(SS, f"check-{m}")
    nadj = "–"
    for p in glob.glob(os.path.join(chk, "spd*.jsonl")):
        n = 0
        for l in open(p):
            try:
                j = json.loads(l)
            except ValueError:
                continue
            if j.get("k") == "adjust":
                n += 1
        nadj = str(n)
    r = usage_rt(os.path.join(chk, "measurements.csv"))
    md.append(f"| `{m}` | {rng(a, 'n')}, {rng(a, 'req')} | {rng(b, 'n')}, {rng(b, 'req')} | {nadj}, {r[0] if r else '–'} |")

# ---------------------------------------------------------------- snapshots
snap = {}
for f in glob.glob(os.path.join(RAW, "snap__*.log")):
    kind, sim, m = parse_name(f)
    d = snap.setdefault((sim, m), {k: [] for k in ("fork_us", "serialize_us", "bytes", "deserialize_us", "restore_us", "continue_ms")})
    d["same"] = True
    for l in open(f, errors="replace"):
        if l.startswith("SNAP") and "phase=run" in l:
            r = kv(l)
            for k in d:
                if k != "same":
                    d[k].append(float(r[k]))
            d["same"] &= r["same_end"] == "true"
if snap:
    md.append("\n### Snapshots of SPD runs (halfway)\n")
    md.append("Median of 10 runs (`spd-bench snapshotAt`): fork = the run and its QVTo state in memory "
              "(`SpdRun::fork`, `QvtoAdjuster::fork_spd`); serialize = snapshot value and postcard bytes incl. "
              "the QVTo section; deserialize = bytes to `Snapshot`; restore = run and QVTo state from it; "
              "continue = the restored run to its end. forks/s = 1 / fork time.\n")
    md.append("| model | flavour, mode | fork µs | forks/s | serialize µs | bytes | deserialize µs | restore µs | continue | restored run ends as uninterrupted |")
    md.append("|---|---|---|---|---|---|---|---|---|---|")
    for (sim, m), d in sorted(snap.items(), key=lambda x: (x[0][1], x[0][0])):
        fk = med(d["fork_us"])
        md.append(f"| `{m}` | {sim.replace('simoxide-', '')} | {fk:.1f} | {1e6 / fk:,.0f} | {med(d['serialize_us']):.0f} | "
                  f"{med(d['bytes']):,.0f} | {med(d['deserialize_us']):.0f} | {med(d['restore_us']):.0f} | "
                  f"{fmt_ms(med(d['continue_ms']))} | {'yes' if d['same'] else 'NO'} |".replace(",", " "))

sf = {}
for f in glob.glob(os.path.join(RAW, "snapfile__*.log")):
    kind, what, m, rep = parse_name(f)
    t = time_line(f)
    d = sf.setdefault((m, what), {"wall": [], "bytes": []})
    if t and t.get("rc") == "0":
        d["wall"].append(float(t["wall_s"]) * 1000)
    for l in open(f + ".err", errors="replace") if os.path.exists(f + ".err") else []:
        mm = re.search(r"\((\d+) bytes\)", l)
        if mm:
            d["bytes"].append(int(mm.group(1)))
if sf:
    md.append("\nSnapshot files through the CLI (exact mode, long run, pause halfway; process wall, median of the processes):\n")
    md.append("| model | uninterrupted run | run to the pause + write snapshot | snapshot bytes | `simoxide resume` to the end |")
    md.append("|---|---|---|---|---|")
    for m in sorted({m for m, _ in sf}):
        g = lambda w: sf.get((m, w), {"wall": [], "bytes": []})
        md.append(f"| `{m}` | {fmt_ms(med(g('full')['wall']))} | {fmt_ms(med(g('pause')['wall']))} | "
                  f"{med(g('pause')['bytes']):,.0f} | {fmt_ms(med(g('resume')['wall']))} |".replace(",", " "))

for md_ in ("exact", "fast"):
    p = os.path.join(RAW, f"fork__simoxide-bench__{md_}.log")
    if os.path.exists(p):
        md.append(f"\n`simoxide-bench fork --reps 5 --mode {md_}` (plain models, halfway through the long run):\n")
        md.append("```text")
        md += [l.rstrip("\n") for l in open(p) if not l.startswith(("#", "TIME"))]
        md.append("```")

# ---------------------------------------------------------------- rules
rules = {}
for f in glob.glob(os.path.join(RAW, "rules__*.log")):
    kind, sim, m, rep = parse_name(f)
    d = rules.setdefault((sim, m), {"wall": [], "rules_ms": [], "prm_ms": [], "n": [], "first": [], "warm": [], "sim_ms": []})
    t = time_line(f)
    if t and t.get("rc") == "0":
        d["wall"].append(float(t["wall_s"]) * 1000)
    err = open(f + ".err", errors="replace").read() if os.path.exists(f + ".err") else ""
    mm = re.search(r"rules: \d+ rule\(s\), (\d+) reconfiguration\(s\).*?([\d.]+) ms \(PRM ([\d.]+) ms\)", err)
    if mm:
        d["n"].append(int(mm.group(1)))
        d["rules_ms"].append(float(mm.group(2)))
        d["prm_ms"].append(float(mm.group(3)))
    mm = re.search(r"wall=([\d.]+)ms", err)
    if mm:
        d["sim_ms"].append(float(mm.group(1)))
    ts = [float(x) for x in re.findall(r"\[refsim\] [^:]+: (\d+) ms", err)]
    if ts:
        d["first"].append(ts[0])
        d["warm"] += ts[1:]
if rules:
    md.append("\n### SimuLizar reconfiguration rules\n")
    md.append("SimOxide: `simoxide run` per process (QVTo ordered mode unless stated): process wall, the run's wall "
              "(after loading and compiling), the time in rules (QVTo runs and the PRM mirror) and the number of "
              "reconfigurations; refsim: `refsim run --seeds 1:11` in one JVM, the first run cold, the others warm "
              "(stock SimuLizar cannot run the rules on the flat classpath: they need refsim's patch P9).\n")
    md.append("| case | simulator | process wall | run wall | reconfigurations | in rules ms (PRM ms) | refsim first / warm run |")
    md.append("|---|---|---|---|---|---|---|")
    for (sim, m), d in sorted(rules.items(), key=lambda x: (x[0][1], x[0][0])):
        md.append(f"| `{m}` | {sim} | {fmt_ms(med(d['wall']))} | {fmt_ms(med(d['sim_ms']))} | "
                  f"{med(d['n']):g} | {med(d['rules_ms']):.1f} ({med(d['prm_ms']):.1f}) | "
                  f"{fmt_ms(med(d['first']))} / {fmt_ms(med(d['warm']))} |")

os.makedirs(OUT, exist_ok=True)
open(os.path.join(OUT, "summary.md"), "w").write("\n".join(md) + "\n")
with open(os.path.join(OUT, "b1.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["kind", "sim", "model", "len", "rep", "key", "value"])
    w.writerows(rows)
print(f"wrote {OUT}/summary.md and b1.csv")
