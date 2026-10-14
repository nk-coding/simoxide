#!/bin/bash
# Smoke test of the SSJ setup (SSJ.md), not a benchmark: short single-core processes, one per simulator and engine,
# on one small model, with seeds 1..N. Shows which engine ran (driver output and the JVM's class-loading log) and
# compares the usage-scenario mean response time of SSJ and DESMO-J.
#   smoke.sh            SimOxide, SimuLizar (flat classpath and OSGi) and SimuCom on DESMO-J and SSJ
#   smoke.sh misarch    can SimuLizar and SimuCom run the generated MiSArch model? (one short run each)
# Env: SMOKE_MODEL (x_espresso), SMOKE_RUNS (10), SMOKE_OSGI (1; 0 = without the OSGi cold runs), SMOKE_CPU (last core), FORCE=1 (skip the quiet-machine check)
set -u
source "$(cd "$(dirname "$0")" && pwd)/common.sh"
LOCK=${SSJ_LOCK-/home/devbox/workspace/misarch-optimization/MEASURING.lock}
if [ -z "${FORCE:-}" ]; then
  [ -n "$LOCK" ] && [ -e "$LOCK" ] && { echo "$LOCK exists: another measurement is running, not now" >&2; exit 3; }
  awk '{exit !($1 < 3)}' /proc/loadavg || { echo "1-min load $(cut -d' ' -f1 /proc/loadavg) >= 3: not now" >&2; exit 3; }
fi
M=${SMOKE_MODEL:-x_espresso}; [ "${1:-}" = misarch ] && M=x_misarch
O=$SSJ_HOME/results/smoke/$M; mkdir -p "$O"
CPU=${SMOKE_CPU:-$(( $(nproc) - 1 ))}
export MEM_FLOOR_KB=${SMOKE_MEM_FLOOR_KB:-1500000}
JO_SMOKE="-Xmx1g -XX:+UseSerialGC"

lock_check() { # stop between steps when another measurement starts
  if [ -z "${FORCE:-}" ] && [ -n "$LOCK" ] && [ -e "$LOCK" ]; then
    echo "smoke: $LOCK appeared, stopping (rerun later; finished steps are kept in $O)" >&2; exit 3
  fi
}

run() { # name sim model len warmup runs [key=value...]
  local name=$1 s=$2 m=$3 len=$4 w=$5 n=$6; shift 6
  local log=$O/$name.log
  if [ -f "$log" ] && grep -aq '^TIME .*rc=0' "$log"; then echo "smoke: $name done before, kept" >&2; return; fi
  lock_check
  rm -f "$log" "$log".*
  local c; c=$(JO="$JO_SMOKE -Xlog:class+load=info:file=$log.classes" cmd "$s" "$m" "$len" "$w" "$n" "$@")
  echo "smoke: $name ($(cut -d' ' -f1 /proc/loadavg) load)" >&2
  echo "# CMD $c" >> "$log"
  # process-tree RSS and thread count every second (SSJ leaves parked process threads behind? see SSJ.md)
  python3 "$C/tools/timeit.py" --sample "$log.rss.csv" --interval 1 --timeout 600 "$log" "taskset -c $CPU $c"
  echo "  $(grep -a '^TIME' "$log" | tail -1)  max_threads=$(awk -F, 'NR > 1 && $4 > m {m = $4} END {print m + 0}' "$log.rss.csv") max_rss_mb=$(awk -F, 'NR > 1 && $3 > m {m = $3} END {printf "%d", m / 1024}' "$log.rss.csv")" >&2
}

engine_evidence() { # log -> the engine classes the JVM loaded, and the driver's own report
  local log=$1
  grep -a -h -o -E '^# ENGINE .*|engine=[A-Za-z0-9_.]+' "$log" 2>/dev/null | sort | uniq -c
  grep -h -o -E '(org\.palladiosimulator\.simulation\.abstractsimengine\.ssj\.SSJExperiment|de\.uka\.ipd\.sdq\.simulation\.abstractsimengine\.desmoj\.DesmoJExperiment|umontreal\.ssj\.simevents\.Simulator|desmoj\.core\.simulator\.Experiment) source: [^ ]*' \
    "$log".classes* 2>/dev/null | sed -E 's#source: .*/([^/]+)$#source: \1#' | sort -u
}

if [ "${1:-}" = misarch ]; then
  T=$(simtime $M short)
  run misarch_simoxide simoxide $M short 0 1
  "$SIMOXIDE_DRV" model="$SSJ_WORK/models/x_misarch_simucom" simTime=$T maxMeas=-1 seed=1 warmup=0 runs=1 \
    > "$O/misarch_simoxide_renamed.log" 2>&1
  run misarch_simulizar simulizar $M short 0 1
  run misarch_simulizar-ssj simulizar-ssj $M short 0 1
  run misarch_simucom simucom $M short 0 1
  # SimuCom on the original names, to document why it gets the renamed copy
  c=$(JO="$JO_SMOKE" cmd simucom $M short 0 1); c=${c/$SSJ_WORK\/models\/x_misarch_simucom/$SSJ_HOME\/models\/x_misarch}
  echo "smoke: misarch_simucom_original_names" >&2
  lock_check; rm -f "$O/misarch_simucom_original_names.log"
  run_logged "$O/misarch_simucom_original_names.log" "taskset -c $CPU $c"
  for f in "$O"/misarch_*.log; do
    echo "== $(basename "$f" .log)"
    grep -a -E '^RESULT' "$f" | grep -o -E 'requests=[^ ]+ mean_rt=[^ ]+|error=.*' | head -3
    grep -a -E 'RUN FAILED|status ' "$f" | head -4
    engine_evidence "$f"
  done
  exit 0
fi

N=${SMOKE_RUNS:-10}
run simoxide simoxide "$M" short 0 "$N" varySeed=true
for e in "" -ssj; do
  run "simulizar$e" "simulizar$e" "$M" short 0 "$N" varySeed=true
  run "simucom$e" "simucom$e" "$M" short 0 3 varySeed=true
  [ "${SMOKE_OSGI:-1}" = 1 ] && run "simulizar$e-osgi" "simulizar$e-osgi" "$M" short 0 1
done
# BenchApp (OSGi) prints its RESULT lines only without quiet=true; one more run each to get them
for e in "" -ssj; do
  [ "${SMOKE_OSGI:-1}" = 1 ] || break
  c=$(JO="$JO_SMOKE" cmd "simulizar$e-osgi" "$M" short 0 1); c=${c/quiet=true/quiet=false}
  echo "smoke: simulizar$e-osgi (verbose)" >&2
  lock_check; rm -f "$O/simulizar$e-osgi-verbose.log"
  run_logged "$O/simulizar$e-osgi-verbose.log" "taskset -c $CPU $c"
done

for f in simoxide simulizar simulizar-ssj simucom simucom-ssj simulizar-osgi simulizar-ssj-osgi; do
  echo "== $f: engine"
  engine_evidence "$O/$f.log"
done
python3 -I - "$O" <<'EOF'
import glob, math, os, re, statistics, sys
O = sys.argv[1]
KV = re.compile(r'(\w+)=(\S+)')
def results(name):
    rows = []
    for line in open(os.path.join(O, name + '.log'), 'rb'):
        line = line.decode('utf-8', 'replace')
        if line.startswith('RESULT '):
            d = dict(KV.findall(line))
            if d.get('phase', 'run') == 'run' and 'error' not in d:
                rows.append(d)
    return rows
print(f"\n{'config':<26}{'runs':>5}{'requests (mean)':>17}{'mean RT (mean of runs)':>24}{'sd':>10}{'wall ms (median)':>18}")
stats = {}
for name in ('simoxide', 'simulizar', 'simulizar-ssj', 'simucom', 'simucom-ssj'):
    if not os.path.exists(os.path.join(O, name + '.log')):
        continue
    r = results(name)
    rt = [float(x['mean_rt']) for x in r if x.get('mean_rt', 'NaN') not in ('NaN', 'nan')]
    req = [float(x['requests']) for x in r if 'requests' in x]
    wall = [float(x['wall_ms']) for x in r if 'wall_ms' in x]
    stats[name] = rt
    sd = statistics.stdev(rt) if len(rt) > 1 else float('nan')
    print(f"{name:<26}{len(r):>5}{statistics.mean(req) if req else float('nan'):>17.1f}"
          f"{statistics.mean(rt) if rt else float('nan'):>24.6f}{sd:>10.4f}{statistics.median(wall) if wall else float('nan'):>18.1f}")
def welch(a, b):
    if len(a) < 2 or len(b) < 2:
        return float('nan')
    va, vb = statistics.variance(a) / len(a), statistics.variance(b) / len(b)
    return (statistics.mean(a) - statistics.mean(b)) / math.sqrt(va + vb) if va + vb > 0 else 0.0
print()
for a, b in (('simulizar-ssj', 'simulizar'), ('simucom-ssj', 'simucom'), ('simulizar', 'simoxide'), ('simucom', 'simoxide')):
    if a in stats and b in stats and stats[a] and stats[b]:
        same = sum(1 for x, y in zip(stats[a], stats[b]) if abs(x - y) <= 1e-6 * max(1, abs(y)))
        print(f"{a} vs {b}: mean RT {statistics.mean(stats[a]):.6f} vs {statistics.mean(stats[b]):.6f}, "
              f"Welch t = {welch(stats[a], stats[b]):.2f}, same-seed runs identical: {same}/{min(len(stats[a]), len(stats[b]))}")
EOF
for e in "" -ssj; do  # OSGi: BenchApp draws a random seed per run, so this is only a plausibility check
  [ -f "$O/simulizar$e-osgi-verbose.log" ] && echo "simulizar$e-osgi: $(grep -a -m1 '^\[bench\] run=' "$O/simulizar$e-osgi-verbose.log" | grep -o 'total=[0-9]*ms.*' | cut -c1-200)"
done
