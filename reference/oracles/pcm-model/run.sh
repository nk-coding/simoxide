#!/bin/bash
# Regenerates the golden dumps of crates/simoxide-model: ./run.sh [models list] [out dir]
# models.txt (name TAB directory; '-' = the bundled default models) is written by collect_models.py;
# rerun it when model directories (e.g. corpus/) change. Each golden/<name>.jsonl is the canonical
# dump (format: crates/simoxide-model/src/canon.rs) of the directory loaded with real EMF 5.2.2.
PRDIR="$("$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/reference/tools/palladio-research.sh")"  # palladio-research work directory
set -e
cd "$(dirname "$0")"
P=${PALLADIO_PLUGINS:-$PRDIR/work-simucom/palladio-5.2.2/plugins}
[ -d build/classes ] || ./build.sh
LIST=${1:-models.txt}
OUT=${2:-golden}
[ -f "$LIST" ] || python3 collect_models.py
# the list's directories are relative to this directory or start with $PALLADIO_RESEARCH; the oracle gets them absolute
RES=build/models.resolved.txt
while IFS= read -r line; do
  case $line in ''|'#'*) printf '%s\n' "$line"; continue ;; esac
  n=${line%%$'\t'*} d=${line#*$'\t'}
  case $d in
    -|/*) ;;
    '$PALLADIO_RESEARCH'/*) d=$PRDIR/${d#'$PALLADIO_RESEARCH'/} ;;
    *) d=$(realpath -m "$d") ;;
  esac
  printf '%s\t%s\n' "$n" "$d"
done < "$LIST" > "$RES"
${JAVA:-java} -cp "build/classes:$(cat build/classpath.txt)" PcmModelOracle "$P" "$RES" "$OUT"
# the dumps of third-party models are git-ignored; their digests are committed (third_party/README.md)
OUT_ABS="$(cd "$OUT" && pwd)"
(cd ../../.. && cargo run -q -p simoxide-thirdparty -- digests "$OUT_ABS")
