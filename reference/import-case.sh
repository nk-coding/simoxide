#!/bin/bash
# Imports a model directory as a corpus-style entry with reference expectations.
#   import-case.sh SRC_DIR NAME [DEST_ROOT]     (DEST_ROOT default: ../corpus-fuzz)
# Copies the model files, run.json, FEATURES.txt, gen.json and REPORT*.txt of SRC_DIR to
# DEST_ROOT/NAME and runs the patched reference (refsim batch) to write DEST_ROOT/NAME/expected/
# (trace.jsonl, tape.jsonl, measurements.csv; files > 256 KiB gzip -n -9). If the reference aborts,
# no expected/ is written; the error goes to DEST_ROOT/NAME/REFERENCE-ERROR.txt and the trace and tape
# up to the abort to DEST_ROOT/NAME/reference-partial/.
set -e
D="$(cd "$(dirname "$0")" && pwd)"
SRC=$1; NAME=$2; ROOT=${3:-$D/../corpus-fuzz}
[ -d "$SRC" ] && [ -n "$NAME" ] || { echo "usage: import-case.sh SRC_DIR NAME [DEST_ROOT]" >&2; exit 2; }
DEST=$ROOT/$NAME
rm -rf "$DEST"; mkdir -p "$DEST"
for f in "$SRC"/*; do
  case "$f" in
    */expected|*/REFERENCE-ERROR.txt) ;;
    *) [ -f "$f" ] && cp "$f" "$DEST/" ;;
  esac
done
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
mkdir -p "$W/in"; ln -s "$(cd "$DEST" && pwd)" "$W/in/$NAME"
"$D/refsim" batch "$W/in" --out "$W/out" > "$W/log" 2>&1 || true
if [ -f "$W/out/$NAME/measurements.csv" ] && ! grep -q "ERROR" "$W/log"; then
  mkdir -p "$DEST/expected"
  cp "$W/out/$NAME"/{trace.jsonl,tape.jsonl,measurements.csv} "$DEST/expected/"
  find "$DEST/expected" -type f -size +256k -exec gzip -n -9 -f {} \;
  echo "$NAME: $(grep -m1 "^$NAME" "$W/log" | cut -c1-150)"
else
  grep -v "^\[refsim\] \(bootstrap\|warm-up\)" "$W/log" | head -40 > "$DEST/REFERENCE-ERROR.txt"
  # the reference's outputs up to the abort (trace ends with the finish line at the abort time)
  mkdir -p "$DEST/reference-partial"
  "$D/refsim" run --model "$DEST" --run-json "$DEST/run.json" --trace "$DEST/reference-partial/trace.jsonl" \
    --tape "$DEST/reference-partial/tape.jsonl" --measurements "$W/partial.csv" > /dev/null 2>&1 || true
  find "$DEST/reference-partial" -type f -size +256k -exec gzip -n -9 -f {} \;
  echo "$NAME: reference failed (see REFERENCE-ERROR.txt)"
fi
