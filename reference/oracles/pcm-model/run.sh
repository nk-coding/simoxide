#!/bin/bash
# Regenerates the golden dumps of crates/simoxide-model: ./run.sh [models list] [out dir]
# models.txt (name TAB directory; '-' = the bundled default models) is written by collect_models.py;
# rerun it when model directories (e.g. corpus/) change. Each golden/<name>.jsonl is the canonical
# dump (format: crates/simoxide-model/src/canon.rs) of the directory loaded with real EMF 5.2.2.
set -e
cd "$(dirname "$0")"
P=${PALLADIO_PLUGINS:-/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2/plugins}
[ -d build/classes ] || ./build.sh
LIST=${1:-models.txt}
OUT=${2:-golden}
[ -f "$LIST" ] || python3 collect_models.py
exec ${JAVA:-java} -cp "build/classes:$(cat build/classpath.txt)" PcmModelOracle "$P" "$LIST" "$OUT"
