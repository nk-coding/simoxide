#!/bin/bash
# det verification beyond the smoke corpus: every given model through the det runner in two fresh JVMs
# and once warm (2 runs in one JVM), plus one stock run. Sequential, niced, one JVM at a time.
#
#   ./verify-det.sh [-o OUT] MODEL_DIR...      default OUT: build/verify; summary OUT/verify.csv
#   ./verify-det.sh -o build/verify ../../corpus/h* ../../corpus/x_* models/probes/*
#
# Simulated time per model: "sim_time" of MODEL_DIR/probe.json, else the end time of the refsim run
# (MODEL_DIR/expected/trace.jsonl[.gz], rounded up), else 100. Seed: probe.json "seed", else the
# corpus run.json "seed", else 1.
# Each JVM runs under a time limit (RUN_TIMEOUT, default 180 s) and a file-size limit (RUN_FILE_LIMIT_KB,
# default 1 GB): zero-time livelocks (closed users with think time 0 whose requests abort) never end.
#
# Columns:
#   det_cold_identical  det, 2 fresh JVMs: trace, tape, spd log, measurements byte-identical
#   det_warm_identical  det, run 2 of a 2-run JVM equals the fresh run
#   stock_vs_det        stock (seeded randomness only) measurements equal det's
#   handler_errors      exceptions thrown by event handlers (the bus logs them and disposes the
#                       subscriber for the rest of the run): count and the first message
#   aborted             UserAborted / JobAborted events in the det trace
set -uo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
OUT=$B/verify
if [ "${1:-}" = -o ]; then OUT=$2; shift 2; fi
mkdir -p "$OUT"
FILES="trace.jsonl tape.jsonl spd.jsonl measurements.csv"

same() { # dirA dirB files... -> "yes" or the first differing file
  local a=$1 b=$2; shift 2
  [ -d "$a" ] && [ -d "$b" ] || { echo "missing"; return; }
  for f in "$@"; do
    [ -f "$a/$f" ] || [ -f "$b/$f" ] || continue
    cmp -s "$a/$f" "$b/$f" || { echo "no($f)"; return; }
  done
  echo yes
}
simtime() { # model dir -> simulated time
  python3 - "$1" <<'EOF'
import gzip, json, math, os, sys
d = sys.argv[1]
p = os.path.join(d, "probe.json")
if os.path.exists(p):
    print(json.load(open(p)).get("sim_time", 100)); sys.exit()
for f, op in (("expected/trace.jsonl", open), ("expected/trace.jsonl.gz", gzip.open)):
    f = os.path.join(d, f)
    if os.path.exists(f):
        last = None
        with op(f, "rt") as h:
            for last in h: pass
        print(max(1, math.ceil(json.loads(last)["t"]))); sys.exit()
print(100)
EOF
}
seed() { python3 -c "
import json,os,sys
for n in ('probe.json','run.json'):
    p=os.path.join(sys.argv[1],n)
    if os.path.exists(p):
        print(json.load(open(p)).get('seed',1)); break
else: print(1)" "$1"; }
field() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))" "$@" 2>/dev/null || echo "?"; }

echo "model,sim_time,seed,events,rows,stoex_draws,math_draws,det_cold_identical,det_warm_identical,stock_vs_det,handler_errors,aborted,error" > "$OUT/verify.csv"
for d in "$@"; do
  d="$(cd "$d" && pwd)"; m=$(basename "$d"); o="$OUT/$m"; rm -rf "$o"; mkdir -p "$o"
  T=$(simtime "$d"); S=$(seed "$d")
  echo "== $m (simTime $T, seed $S)" >&2
  run() { local tag=$1 mode=$2; shift 2
    ( ulimit -f "${RUN_FILE_LIMIT_KB:-1048576}"
      timeout "${RUN_TIMEOUT:-180}" "$H/run.sh" $mode -model "$d" -simTime "$T" -out "$o/$tag" "$@" ) > "$o/$tag.log" 2>&1
    local rc=$?
    [ $rc -eq 0 ] || echo "exit $rc" >> "$o/$tag.log"
    [ $rc -eq 124 ] || [ $rc -eq 153 ] && { echo "limit" > "$o/$tag.limit"; rm -f "$o/$tag/trace.jsonl" "$o/$tag/tape.jsonl"; }
    return 0; }
  run det-a --det -seed "$S"
  run det-b --det -seed "$S"
  run det-warm --det -seed "$S" -runs 2
  run stock --stock -seed "$S"
  cold=$(same "$o/det-a" "$o/det-b" $FILES)
  warm=$(same "$o/det-a" "$o/det-warm/run-1" $FILES)
  svd=$(same "$o/stock" "$o/det-a" measurements.csv)
  ev=$(field "$o/det-a/summary.json" "d['events']"); rows=$(field "$o/det-a/summary.json" "d['rows']")
  st=$(field "$o/det-a/summary.json" "d['instrument']['stoex_draws']")
  ma=$(field "$o/det-a/summary.json" "d['instrument']['math_draws']")
  err=$(field "$o/det-a/summary.json" "d.get('error') or (d['instrument'] or {}).get('error') or ''" | tr ',\n' ';;')
  [ -f "$o/det-a.limit" ] && err="time or size limit (livelock?)"
  [ "$err" = "?" ] && err=""
  [ -f "$o/det-a/summary.json" ] || [ -n "$err" ] || err="no result: $(grep -m1 -E 'Caused by|Exception' "$o/det-a.log" | cut -c1-150 | tr ',' ';')"
  he=$(grep -c "An error occurred" "$o/det-a.log"; true)
  [ "$he" -gt 0 ] && he="$he: $(grep -m1 "An error occurred" "$o/det-a.log" | sed 's/.*An error occurred: //' | cut -c1-120 | tr ',' ';')"
  ab=$(grep -c -E '"ev":"(UserAborted|JobAborted)"' "$o/det-a/trace.jsonl" 2>/dev/null; true)
  echo "$m,$T,$S,$ev,$rows,$st,$ma,$cold,$warm,$svd,$he,$ab,$err" | tee -a "$OUT/verify.csv" >&2
  find "$o" -name trace.jsonl -size +50M -delete
done
echo "summary: $OUT/verify.csv" >&2
