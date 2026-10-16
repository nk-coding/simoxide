#!/bin/sh
# Builds and runs the parse-only oracle against Eclipse QVTo's jars (see ParseOracle.java).
# Usage: run.sh [-time [repeat]] < file-list > problems.tsv
# LIB: jar directory (default: the reference runner's qvto-3.11.0 set). OUT: class directory.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)
LIB=${LIB:-$ROOT/reference/runner/lib/qvto-3.11.0}
[ -d "$LIB" ] || LIB=/home/devbox/workspace/qvto-rs/reference/runner/lib/qvto-3.11.0
OUT=${OUT:-${CARGO_TARGET_DIR:-$ROOT/target}/parse-oracle}
CP=$(ls "$LIB"/*.jar | tr '\n' ':')
mkdir -p "$OUT"
if [ ! -f "$OUT/ParseOracle.class" ] || [ "$HERE/ParseOracle.java" -nt "$OUT/ParseOracle.class" ]; then
  nice -n 19 javac -nowarn -d "$OUT" -cp "$CP" "$HERE/ParseOracle.java"
fi
exec nice -n 19 java -Xmx1g -cp "$OUT:$CP" ParseOracle "$@"
