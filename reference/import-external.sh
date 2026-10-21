#!/bin/bash
# Imports the external example models listed in external-models.txt into a corpus dir (default ../corpus):
# copies the model files (flat, relative hrefs), adds the refsim default monitors and writes run.json
# (seed 1, network flags explicit). Usage: import-external.sh [corpusDir] [name-regex]
#
# Sources under third_party/ are git submodules (third_party/README.md): their model files are not
# kept as copies; after the import each one is replaced by a relative link to the upstream file of
# the same name (the import's EMF re-save loads into the same model, see third_party/README.md).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRDIR="$("$ROOT/reference/tools/palladio-research.sh")"  # palladio-research (moved to old/)
cd "$(dirname "$0")"
OUT=${1:-../corpus}; RE=${2:-.}
BASE=$PRDIR
LOG=$(mktemp)
grep -v '^#' external-models.txt | grep -E "^($RE)" | while IFS='|' read -r name src opts; do
  [ -z "$name" ] && continue
  case "$src" in
    third_party/*)
      sub=$(echo "$src" | cut -d/ -f1-2)
      if [ -z "$(ls -A "$ROOT/$sub" 2>/dev/null)" ]; then echo "FAIL $name: $sub not checked out (git submodule update --init)"; continue; fi
      files=$(echo "$src" | tr ',' '\n' | sed "s|^|$ROOT/|" | paste -sd,) ;;
    *) files=$(echo "$src" | tr ',' '\n' | sed "s|^|$BASE/|" | paste -sd,) ;;
  esac
  max=50; extra=()
  for o in $(echo "$opts" | tr ',' ' '); do
    case "$o" in max=*) max=${o#max=};; skip-external-calls) extra+=(--skip-external-calls);; esac
  done
  rj=$(printf '{\n  "seed": 1,\n  "max_measurements": %s,\n  "max_sim_time": -1,\n  "simulate_linking_resources": false,\n  "simulate_throughput_of_linking_resources": true\n}' "$max")
  mkdir -p "$OUT/$name"; find "$OUT/$name" -maxdepth 1 \( -type f -o -type l \) ! -name README.md -delete
  if ./refsim import "$files" "$OUT/$name" --run-json "$rj" "${extra[@]}" >"$LOG" 2>&1; then
    case "$src" in
      third_party/*)
        first=$(echo "$src" | cut -d, -f1); dir="$ROOT/$first"; [ -d "$dir" ] || dir=$(dirname "$dir")
        for f in "$OUT/$name"/*; do
          b=$(basename "$f")
          case "$b" in refsim.*|run.json|README.md) continue;; esac
          up=$(find "$dir" -name "$b" -type f | head -2)
          if [ "$(echo "$up" | wc -l)" != 1 ] || [ -z "$up" ]; then echo "FAIL $name: no unique upstream file for $b"; continue 2; fi
          ln -sfn "$(realpath --relative-to="$(dirname "$(realpath "$f")")" "$up")" "$f"
        done ;;
    esac
    echo "ok   $name"
  else echo "FAIL $name: $(grep -m1 'FAILED' "$LOG" | cut -c1-200)"; fi
done
rm -f "$LOG"
