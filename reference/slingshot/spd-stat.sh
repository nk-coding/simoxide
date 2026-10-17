#!/bin/bash
# V4 (DESIGN-SPD.md §4.2, phase P3): SimOxide's Slingshot flavour with scaling policies against the
# stock Slingshot runner, seeds FROM..TO per scenario, then tools/spd_stat.py (adjustment counts and
# times, target-group size, response time per window) and tools/compare_stat.py (every series).
#   ./spd-stat.sh OUT_DIR SIMOXIDE_BINARY [FROM TO] [case...]
# SIMOXIDE_BINARY: a build with `--features reconfiguration` (adjustments). Cases: models/<case>,
# simulated time SIM_TIME (default 600). SimOxide runs with Slingshot's quirks (`--quirks fast
# --spd-quirks slingshot`, what stock Slingshot does); SX_FLAGS adds flags. One JVM at a time,
# niced, -Xmx2g, under prlimit (MALLOC_ARENA_MAX=2 lets the JVM create its threads there).
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=$(mkdir -p "$1" && cd "$1" && pwd); BIN=$2; FROM=${3:-1}; TO=${4:-30}; shift 4 || shift $#
CASES=${*:-spd_v4_cpu spd_v4_rt}
T=${SIM_TIME:-600}
export MALLOC_ARENA_MAX=2
for c in $CASES; do
  m="$H/models/$c"
  for s in $(seq "$FROM" "$TO"); do
    r="$OUT/ref/$c/$s"; mkdir -p "$OUT/ref/$c"
    if [ ! -f "$r/spd.jsonl" ]; then
      prlimit --as=10000000000 -- "$H/run.sh" --stock -model "$m" -out "$r" -simTime "$T" -seed "$s" > "$r.log" 2>&1 || true
      rm -f "$r/trace.jsonl" "$r/tape.jsonl" "$r/bus.jsonl"
    fi
    x="$OUT/sx/$c/$s"; mkdir -p "$x"
    prlimit --as=10000000000 -- nice -n 10 "$BIN" run --model "$m" --flavour slingshot --quirks fast \
      --spd-quirks slingshot ${SX_FLAGS:-} --max-sim-time "$T" --seed "$s" --spd-log "$x/spd.jsonl" \
      --measurements "$x/measurements.csv" > /dev/null 2> "$x.log"
  done
  echo "== $c"
  python3 "$H/tools/spd_stat.py" --sim-time "$T" --csv "$OUT/$c.spd.tsv" "$OUT"/ref/"$c"/*/ -- "$OUT"/sx/"$c"/*/ | tail -n 1 || true
  python3 "$H/tools/compare_stat.py" --csv "$OUT/$c.series.tsv" "$OUT"/ref/"$c"/*/ -- "$OUT"/sx/"$c"/*/ | tail -n 1 || true
done
