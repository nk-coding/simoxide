#!/bin/bash
# V4 (DESIGN-SPD.md §4.2, phase P3): SimOxide's Slingshot flavour with scaling policies against the
# stock Slingshot runner, seeds FROM..TO per scenario, then tools/spd_stat.py (adjustment counts and
# times, target-group size, response time per window) and tools/compare_stat.py (every series).
#   ./spd-stat.sh OUT_DIR SIMOXIDE_BINARY [FROM TO] [case...]
# SIMOXIDE_BINARY: a build with `--features reconfiguration` (adjustments). Cases: models/<case>,
# simulated time SIM_TIME (default 600). SimOxide runs with Slingshot's quirks (`--quirks fast
# --spd-quirks slingshot`, what stock Slingshot does); SX_FLAGS adds flags. One JVM at a time,
# niced, -Xmx2g, under prlimit (MALLOC_ARENA_MAX=2 lets the JVM create its threads there).
# With CONTROL=1 also the reference's seeds TO+1..2*TO-FROM+1 and the same comparisons of the
# reference against itself (the methodology's false-rejection rate).
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=$(mkdir -p "$1" && cd "$1" && pwd); BIN=$2; FROM=${3:-1}; TO=${4:-30}; shift 4 || shift $#
CASES=${*:-spd_v4_cpu spd_v4_rt}
T=${SIM_TIME:-600}
export MALLOC_ARENA_MAX=2
N=$((TO - FROM + 1))
LAST=$TO; [ "${CONTROL:-0}" = 1 ] && LAST=$((TO + N))
for c in $CASES; do
  m="$H/models/$c"
  for s in $(seq "$FROM" "$LAST"); do
    r="$OUT/ref/$c/$s"; mkdir -p "$OUT/ref/$c"
    if [ ! -f "$r/spd.jsonl" ]; then
      prlimit --as=10000000000 -- "$H/run.sh" --stock -model "$m" -out "$r" -simTime "$T" -seed "$s" > "$r.log" 2>&1 || true
      rm -f "$r/trace.jsonl" "$r/tape.jsonl" "$r/bus.jsonl"
    fi
    [ "$s" -le "$TO" ] || continue
    x="$OUT/sx/$c/$s"; mkdir -p "$x"
    prlimit --as=10000000000 -- nice -n 10 "$BIN" run --model "$m" --flavour slingshot --quirks fast \
      --spd-quirks slingshot ${SX_FLAGS:-} --max-sim-time "$T" --seed "$s" --spd-log "$x/spd.jsonl" \
      --measurements "$x/measurements.csv" > /dev/null 2> "$x.log"
  done
  ref=(); ctl=(); sx=()
  for s in $(seq "$FROM" "$TO"); do ref+=("$OUT/ref/$c/$s"); sx+=("$OUT/sx/$c/$s"); done
  for s in $(seq $((TO + 1)) "$LAST"); do ctl+=("$OUT/ref/$c/$s"); done
  echo "== $c"
  echo -n "spd: "; python3 "$H/tools/spd_stat.py" --sim-time "$T" --csv "$OUT/$c.spd.tsv" "${ref[@]}" -- "${sx[@]}" | tail -n 1 || true
  echo -n "series: "; python3 "$H/tools/compare_stat.py" --created-from "$m" --csv "$OUT/$c.series.tsv" "${ref[@]}" -- "${sx[@]}" | tail -n 1 || true
  if [ ${#ctl[@]} -gt 0 ]; then
    echo -n "control spd: "; python3 "$H/tools/spd_stat.py" --sim-time "$T" --csv "$OUT/$c.control.spd.tsv" "${ref[@]}" -- "${ctl[@]}" | tail -n 1 || true
    echo -n "control series: "; python3 "$H/tools/compare_stat.py" --created-from "$m" --csv "$OUT/$c.control.series.tsv" "${ref[@]}" -- "${ctl[@]}" | tail -n 1 || true
  fi
done
