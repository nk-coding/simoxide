#!/bin/bash
# Regenerates corpus/*/expected/{trace.jsonl,tape.jsonl,measurements.csv} with the patched reference.
# Files larger than 256 KiB are stored gzip'ed (gzip -n -9, deterministic) as <file>.gz.
#   regen-expected.sh [--only a,b]        regenerate (all models, or the listed ones)
#   regen-expected.sh --check [--only ..]  re-run and compare with the stored expected outputs (no writes)
set -e
cd "$(dirname "$0")"
CORPUS=../corpus
if [ "$1" = "--check" ]; then shift; exec ./refsim batch "$CORPUS" --check "$@"; fi
ONLY=""; [ "$1" = "--only" ] && ONLY="$2"
for d in "$CORPUS"/*/; do
  n=$(basename "$d"); [ -f "$d/run.json" ] || continue
  if [ -n "$ONLY" ] && ! echo ",$ONLY," | grep -q ",$n,"; then continue; fi
  rm -rf "$d/expected"
done
if [ -n "$ONLY" ]; then ./refsim batch "$CORPUS" --in-place --only "$ONLY"; else ./refsim batch "$CORPUS" --in-place; fi
find "$CORPUS" -path '*/expected/*' -type f ! -name '*.gz' -size +256k -exec gzip -n -9 -f {} \;
# re-check (also measures warm run times) and rebuild corpus/INDEX.md
LOG=$(mktemp); ./refsim batch "$CORPUS" --check > "$LOG"; tail -n 1 "$LOG"
python3 tools/mkindex.py "$LOG" "$CORPUS"; rm -f "$LOG"
du -sh "$CORPUS"
