#!/bin/bash
# Imports the external example models listed in external-models.txt into a corpus dir (default ../corpus):
# copies the model files (flat, relative hrefs), adds the refsim default monitors and writes run.json
# (seed 1, network flags explicit). Usage: import-external.sh [corpusDir] [name-regex]
cd "$(dirname "$0")"
OUT=${1:-../corpus}; RE=${2:-.}
BASE=/home/devbox/workspace/palladio-research
LOG=$(mktemp)
grep -v '^#' external-models.txt | grep -E "^($RE)" | while IFS='|' read -r name src opts; do
  [ -z "$name" ] && continue
  files=$(echo "$src" | tr ',' '\n' | sed "s|^|$BASE/|" | paste -sd,)
  max=50; extra=()
  for o in $(echo "$opts" | tr ',' ' '); do
    case "$o" in max=*) max=${o#max=};; skip-external-calls) extra+=(--skip-external-calls);; esac
  done
  rj=$(printf '{\n  "seed": 1,\n  "max_measurements": %s,\n  "max_sim_time": -1,\n  "simulate_linking_resources": false,\n  "simulate_throughput_of_linking_resources": true\n}' "$max")
  mkdir -p "$OUT/$name"; find "$OUT/$name" -maxdepth 1 -type f -delete
  if ./refsim import "$files" "$OUT/$name" --run-json "$rj" "${extra[@]}" >"$LOG" 2>&1; then echo "ok   $name"; else echo "FAIL $name: $(grep -m1 'FAILED' "$LOG" | cut -c1-200)"; fi
done
rm -f "$LOG"
