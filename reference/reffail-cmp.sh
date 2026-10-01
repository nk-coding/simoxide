#!/bin/bash
# reffail-cmp.sh SIMOXIDE CASEDIR...
# For models whose reference run aborts (fuzz --save-ref-failures, REPORT.reffail.txt): runs the
# reference (partial trace up to the abort) and the candidate, and prints the first differing trace
# line. If the first difference is the reference's final stop/finish lines, the candidate followed
# the reference exactly up to the abort.
D="$(cd "$(dirname "$0")" && pwd)"
P=$1; shift
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
for d in "$@"; do
  n=$(basename "$d")
  "$D/refsim" run --model "$d" --run-json "$d/run.json" --trace $W/r.jsonl --tape $W/r.tape --measurements $W/r.csv > /dev/null 2>&1
  "$P" run --model "$d" --run-json "$d/run.json" --name "$n" --trace $W/p.jsonl --tape $W/p.tape --measurements $W/p.csv > $W/p.out 2>&1
  python3 - "$n" "$W" <<'PY'
import sys
n, w = sys.argv[1], sys.argv[2]
r = [l for l in open(w + '/r.jsonl').read().split('\n') if l]
p = [l for l in open(w + '/p.jsonl').read().split('\n') if l]
last = open(w + '/p.out').read().strip().split('\n')[-1][:120]
k = next((i + 1 for i, (a, b) in enumerate(zip(r[1:], p[1:])) if a != b), None)
ref = r[k][:110] if k is not None else ''
cand = p[k][:110] if k is not None and k < len(p) else ''
print(f"{n}: reference {len(r)} lines, candidate {len(p)} lines ({last}); first diff line {k}: reference={ref} | candidate={cand}")
PY
done
