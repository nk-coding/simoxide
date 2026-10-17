#!/bin/bash
# Runs SimOxide's Slingshot-exact mode (--flavour slingshot-exact) on corpus-ss cases and compares
# with their det reference outputs (docs/spec/slingshot/exact.md SS-X-1, SS-X-8).
#
#   tools/compare_exact.sh [-b SIMOXIDE] [-o OUT] [-c CORPUS] [NAME...]
#                                        default: every case of corpus-ss/ (-c corpus-spd: that one)
#
# Per case one line: E (level-E projection), M (level M), T (tape projection with positions),
# C (measurements.csv): "=" equal, "x" different, "-" no reference; then the first differing
# projection line. Outputs in OUT/<name>/ (default build/exact).
#
# A case whose model directory has a `.spd` runs with its scaling policies (all SPD quirks, as
# the reference) and writes the decision log OUT/<name>/spd.jsonl. Their adjustments run only in
# a binary with the `reconfiguration` feature (e.g. `cargo build --release -p simoxide-cli
# --features exact,hotspot-math,reconfiguration`); with another binary the line says so.
set -uo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
R="$(cd "$H/../.." && pwd)"
BIN="$R/target/release/simoxide"
OUT="$H/build/exact"
CORPUS=corpus-ss
while [ $# -gt 0 ]; do
  case "$1" in
    -b) BIN=$2; shift 2 ;;
    -o) OUT=$2; shift 2 ;;
    -c) CORPUS=$2; shift 2 ;;
    *) break ;;
  esac
done
names=("$@")
[ ${#names[@]} -gt 0 ] || names=($(cd "$R/$CORPUS" && ls -d */ | tr -d /))
field() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])" "$1" "$2"; }
levm() { grep -E '^\{"ev":"(job_abort|user_abort|meas|spd_[a-z]*|stop)"' "$1"; }
ok=0; n=0
for c in "${names[@]}"; do
  d="$R/$CORPUS/$c"; e="$d/expected"; o="$OUT/$c"; rm -rf "$o"; mkdir -p "$o"
  m="$R/$(field "$d/case.json" model)"; T=$(field "$d/case.json" sim_time); S=$(field "$d/case.json" seed)
  spd=(); compgen -G "$m/*.spd" > /dev/null && spd=(--spd-log "$o/spd.jsonl")
  "$BIN" run --model "$m" --flavour slingshot-exact --seed "$S" --max-sim-time "$T" "${spd[@]}" \
    --trace "$o/projection.jsonl" --tape "$o/tape-projection.jsonl" --measurements "$o/measurements.csv" \
    > "$o/log" 2>&1
  rc=$?
  note=""; grep -q "adjustments are not executed" "$o/log" && note=" (SPD adjustments not executed: build with reconfiguration)"
  st() { [ -f "$e/$1" ] || { echo -n "-"; return; }; cmp -s "$e/$1" "$o/$1" && echo -n "=" || echo -n "x"; }
  E=$(st projection.jsonl); TT=$(st tape-projection.jsonl); C=$(st measurements.csv)
  M="-"; [ -f "$e/projection.jsonl" ] && { cmp -s <(levm "$e/projection.jsonl") <(levm "$o/projection.jsonl") && M="=" || M="x"; }
  first=""
  if [ "$E" = x ]; then
    first=$(cmp "$e/projection.jsonl" "$o/projection.jsonl" 2>&1 | head -1 | sed 's/.*line //')
  fi
  n=$((n+1)); [ "$E$M$TT$C" = "====" ] && ok=$((ok+1))
  printf '%-40s E%s M%s T%s C%s rc=%s %s%s\n' "$c" "$E" "$M" "$TT" "$C" "$rc" "${first:+first diff line $first}" "$note"
done
echo "all four equal: $ok of $n"
