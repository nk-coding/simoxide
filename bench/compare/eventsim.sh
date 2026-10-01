#!/bin/bash
# EventSim 5.1.0 (archived, Palladio 5.1, Java 11) via palladio-research/work-slingshot/run-eventsim.sh,
# converted to the RESULT/PAR lines of the other drivers. Args: model=DIR simTime=T warmup=W runs=R
# EventSim has no usage-scenario response-time probe in its default instrumentation: requests = number of
# usage-scenario inter-departure times + 1, mean_rt = NaN. No seed (config seed not wired). No barrier.
C="$(cd "$(dirname "$0")" && pwd)"; source "$C/env.sh"
declare -A A; for kv in "$@"; do A[${kv%%=*}]=${kv#*=}; done
W=${A[warmup]:-0}; N=${A[runs]:-1}
# duration=S (sustained): as many runs as fit, the JVM is stopped after S seconds (+ start-up allowance)
TO=""; [ -n "${A[duration]:-}" ] && { N=1000000; TO="timeout --signal=KILL $(( ${A[duration]%.*} + 8 ))"; }
JAVA11=$(ls -d $WSL/jdk/jdk-11*/bin/java | head -1)
JAVA=$JAVA11 JAVA_OPTS="${JAVA_OPTS:--Xmx4g}" $TO $WSL/run-eventsim.sh -model "${A[model]}" -simTime "${A[simTime]}" \
  -runs $((W + N)) -log ERROR -showResults true 2>&1 | python3 -c '
import re, sys
model, simtime, warm = sys.argv[1], float(sys.argv[2]), int(sys.argv[3])
cur = None; rows = []
for line in sys.stdin:
    m = re.match(r"run (\d+) \[[^]]*\]: load=(\d+)ms init=(\d+)ms sim=(\d+)ms measurements=(\d+)", line)
    if m:
        cur = [int(m.group(1)), int(m.group(2)) + int(m.group(3)) + int(m.group(4)), int(m.group(4)), -1]
        rows.append(cur); continue
    m = re.search(r"INTER_DEPARTURE_TIME @ UsageScenarioImpl:[^:]*: n=(\d+)", line)
    if m and cur: cur[3] = int(m.group(1)) + 1
    if "FAILED" in line: print("ERROR " + line.strip())
tot = 0
for i, wall, sim, req in rows:
    ph = "warm" if i < warm else "run"
    k = i if i < warm else i - warm
    if ph == "run": tot += wall
    # t_end_ms: sum of the measured runs (load+init+sim; the app prints no timestamps)
    te = tot if ph == "run" else -1
    print(f"RESULT sim=eventsim model={model} phase={ph} run={k} wall_ms={wall:.3f} sim_ms={sim:.3f} requests={req} mean_rt=NaN sim_end={simtime:.3f} events=-1 t_end_ms={te:.1f}")
n = len(rows) - warm
if n > 0: print(f"PAR sim=eventsim model={model} threads=1 runs={n} wall_ms={tot:.1f} runs_per_s={n / (tot / 1000):.3f} end_epoch_ms=0")
' "$(basename "${A[model]}")" "${A[simTime]}" "$W"
