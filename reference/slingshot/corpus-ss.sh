#!/bin/bash
# The Slingshot-exact corpus corpus-ss/ (DESIGN-SPD.md X1, docs/spec/slingshot/exact.md SS-X-8): every
# case is a model directory plus a seed and a simulated time (corpus-ss/<name>/case.json); its
# expected/ holds the det runner's outputs, which SimOxide's Slingshot-exact mode must reproduce byte
# for byte (SS-X-1).
#
#   ./corpus-ss.sh [NAME...]            run det once per case and (re)write corpus-ss/<name>/expected/
#   ./corpus-ss.sh --check [NAME...]    run det again (a fresh JVM) and compare with expected/
#
# expected/: measurements.csv, projection.jsonl (level E; level M is its subset), tape-projection.jsonl
# (tools/project.py), errors.txt (handler exceptions, tools/errors.py), summary.json (sorted keys),
# spd.jsonl (SPD runs), trace.jsonl.gz (the full bus trace, gzip -n; for debugging, not compared by
# SimOxide) and, for runs that abort before t = 0, error.txt.
# One JVM at a time, niced (run.sh: nice 19, -Xmx2g).
set -uo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
R="$(cd "$H/../.." && pwd)"
C="$R/corpus-ss"
check=0
[ "${1:-}" = --check ] && { check=1; shift; }
names=("$@")
[ ${#names[@]} -gt 0 ] || names=($(cd "$C" && ls -d */ | tr -d /))
W="$B/corpus-ss"; mkdir -p "$W"
FILES="measurements.csv projection.jsonl tape-projection.jsonl errors.txt summary.json spd.jsonl error.txt"
fail=0
field() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])" "$1" "$2"; }
for n in "${names[@]}"; do
  c="$C/$n/case.json"; [ -f "$c" ] || { echo "no case $n" >&2; fail=1; continue; }
  m="$R/$(field "$c" model)"; T=$(field "$c" sim_time); S=$(field "$c" seed)
  o="$W/$n"; rm -rf "$o" "$W/$n.log"
  ( ulimit -f "${RUN_FILE_LIMIT_KB:-1048576}"
    timeout "${RUN_TIMEOUT:-300}" "$H/run.sh" -model "$m" -simTime "$T" -seed "$S" -out "$o" ) > "$W/$n.log" 2>&1
  mkdir -p "$o"
  [ -f "$o/trace.jsonl" ] && python3 "$H/tools/project.py" "$o" > /dev/null
  python3 "$H/tools/errors.py" "$W/$n.log" > "$o/errors.txt"
  python3 - "$o" <<'EOF'
import json, os, sys
o = sys.argv[1]
p = os.path.join(o, "summary.json")
if os.path.exists(p):
    d = json.load(open(p))
    json.dump(d, open(p, "w"), sort_keys=True)
    err = d.get("error") or (d.get("instrument") or {}).get("error")
    if err:
        open(os.path.join(o, "error.txt"), "w").write(str(err) + "\n")
s = os.path.join(o, "spd.jsonl")
if os.path.exists(s) and os.path.getsize(s) == 0:
    os.remove(s)
EOF
  [ -f "$o/summary.json" ] || grep -m1 -E 'Caused by|Exception' "$W/$n.log" | cut -c1-300 > "$o/error.txt"
  e="$C/$n/expected"
  if [ $check = 1 ]; then
    r=ok
    for f in $FILES; do
      [ -f "$o/$f" ] || [ -f "$e/$f" ] || continue
      cmp -s "$o/$f" "$e/$f" || { r="differs: $f"; break; }
    done
    if [ "$r" = ok ] && [ -f "$e/trace.jsonl.gz" ]; then
      zcat "$e/trace.jsonl.gz" | cmp -s - "$o/trace.jsonl" || r="differs: trace.jsonl"
    fi
    [ "$r" = ok ] || fail=1
    echo "$n: $r"
  else
    rm -rf "$e"; mkdir -p "$e"
    for f in $FILES; do [ -f "$o/$f" ] && cp "$o/$f" "$e/"; done
    [ -f "$o/trace.jsonl" ] && gzip -n -c "$o/trace.jsonl" > "$e/trace.jsonl.gz"
    echo "$n: $(wc -l < "$e/projection.jsonl" 2>/dev/null || echo 0) projected lines, $(wc -l < "$e/errors.txt") handler errors$([ -f "$e/error.txt" ] && echo ", error: $(head -c 100 "$e/error.txt")")"
  fi
done
exit $fail
