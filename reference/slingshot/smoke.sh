#!/bin/bash
# Smoke corpus of the Slingshot reference runner: every model through the runner, determinism checks
# and per-run times (input for benchmark B1). Sequential, niced, one JVM at a time.
#
#   ./smoke.sh [OUT]          default OUT: build/smoke; summary: OUT/smoke.csv (copied to results/)
#   MODELS="x_ss_minimal spd_minimal" ./smoke.sh
#
# Per model:
#   det-a, det-b   det mode, seed 1, instrumented, two fresh JVMs        -> identical? (all output files)
#   det-warm       det mode, seed 1, instrumented, 3 runs in one JVM      -> each identical to det-a?
#   det-noinst     det mode, seed 1, no instrumentation                   -> measurements identical to det-a?
#   stock-a/b      unpatched nightly, seed 1 (seeded randomness only)     -> identical? (expected: not always)
#   time-cold      det mode, no instrumentation, 1 run, fresh JVM         -> run wall + JVM wall
#   time-warm      det mode, no instrumentation, 6 runs in one JVM         -> median wall of runs 1..5
set -uo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
R="$(cd "$H/../.." && pwd)"
OUT=${1:-$B/smoke}
mkdir -p "$OUT"

# model -> directory and simulated time (s). Corpus models use bench/compare's short times where it has
# them; the Slingshot variants of bench/compare/models where Slingshot needs explicit acquire demands.
declare -A DIR=(
  [x_ss_minimal]=$R/corpus/x_ss_minimal
  [x_ss_minimalexample]=$R/corpus/x_ss_minimalexample
  [x_ss_minimalexample_open_ps]=$R/corpus/x_ss_minimalexample_open_ps
  [x_ss_delay]=$R/corpus/x_ss_delay
  [x_ss_mediastore]=$R/corpus/x_ss_mediastore
  [x_espresso]=$R/corpus/x_espresso
  [x_sl_mediastore]=$R/bench/compare/models/x_sl_mediastore_ss
  [spd_minimal]=$H/models/spd_minimal
  [spd_scale_in]=$H/models/spd_scale_in
)
declare -A SIMT=(
  [x_ss_minimal]=1000 [x_ss_minimalexample]=1000 [x_ss_minimalexample_open_ps]=1000 [x_ss_delay]=1000
  [x_ss_mediastore]=200000 [x_espresso]=100 [x_sl_mediastore]=100000 [spd_minimal]=100 [spd_scale_in]=150
)
MODELS=${MODELS:-"spd_minimal spd_scale_in x_ss_minimal x_ss_minimalexample x_ss_minimalexample_open_ps x_ss_delay x_espresso x_ss_mediastore x_sl_mediastore"}
FILES="trace.jsonl tape.jsonl spd.jsonl measurements.csv"

same() { # dirA dirB files... -> "yes" or "no(<first differing file>)"
  local d; d=$(oraclekit_first_diff "$@")
  if [ -z "$d" ]; then echo yes; else echo "no($d)"; fi
}
field() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))" "$@" 2>/dev/null || echo "?"; }
median() { sort -n | awk '{a[NR]=$1} END {if (NR==0) print "?"; else print a[int((NR+1)/2)]}'; }

echo "model,sim_time,events,rows,trace_lines,stoex_draws,math_draws,spd_requests,det_cold_identical,det_warm_identical,instrumentation_neutral,stock_identical,stock_vs_det_measurements,cold_run_ms,cold_jvm_ms,warm_run_ms,warm_sim_ms,error" > "$OUT/smoke.csv"
for m in $MODELS; do
  d=${DIR[$m]}; T=${SIMT[$m]}; o="$OUT/$m"; rm -rf "$o"; mkdir -p "$o"
  echo "== $m (simTime $T)" >&2
  run() { local tag=$1; shift; "$H/run.sh" "$@" -model "$d" -simTime "$T" -out "$o/$tag" > "$o/$tag.log" 2>&1; }
  run det-a -seed 1
  run det-b -seed 1
  run det-warm -seed 1 -runs 3
  run det-noinst -seed 1 -instrument false
  run stock-a --stock -seed 1
  run stock-b --stock -seed 1
  t0=$(date +%s%N)
  run time-cold -seed 1 -instrument false -measurements false
  t1=$(date +%s%N)
  run time-warm -seed 1 -instrument false -measurements false -runs 6

  cold=$(same "$o/det-a" "$o/det-b" $FILES)
  warm=yes
  for i in 0 1 2; do r=$(same "$o/det-a" "$o/det-warm/run-$i" $FILES); [ "$r" = yes ] || warm=$r; done
  neutral=$(same "$o/det-a" "$o/det-noinst" measurements.csv)
  stock=$(same "$o/stock-a" "$o/stock-b" $FILES)
  svd=$(same "$o/stock-a" "$o/det-a" measurements.csv)
  ev=$(field "$o/det-a/summary.json" "d['events']"); rows=$(field "$o/det-a/summary.json" "d['rows']")
  st=$(field "$o/det-a/summary.json" "d['instrument']['stoex_draws']")
  ma=$(field "$o/det-a/summary.json" "d['instrument']['math_draws']")
  rq=$(field "$o/det-a/summary.json" "d['instrument']['spd_requests']")
  err=$(field "$o/det-a/summary.json" "d.get('error') or (d['instrument'] or {}).get('error') or ''")
  tl=$(wc -l < "$o/det-a/trace.jsonl" 2>/dev/null || echo "?")
  coldrun=$(awk -F, 'NR==2 {print $7}' "$o/time-cold/timings.csv" 2>/dev/null)
  coldjvm=$(( (t1 - t0) / 1000000 ))
  warmrun=$(awk -F, 'NR>2 {print $7}' "$o/time-warm/timings.csv" 2>/dev/null | median)
  warmsim=$(awk -F, 'NR>2 {print $5}' "$o/time-warm/timings.csv" 2>/dev/null | median)
  echo "$m,$T,$ev,$rows,$tl,$st,$ma,$rq,$cold,$warm,$neutral,$stock,$svd,$coldrun,$coldjvm,$warmrun,$warmsim,$err" | tee -a "$OUT/smoke.csv" >&2
  # keep the outputs small: traces of large runs are not needed after the comparison
  find "$o" -name trace.jsonl -size +50M -delete
done
mkdir -p "$H/results" && cp "$OUT/smoke.csv" "$H/results/smoke.csv"
echo "summary: $H/results/smoke.csv" >&2
