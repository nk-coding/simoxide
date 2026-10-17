#!/bin/sh
# Builds and runs a Java tool of this directory against Eclipse QVTo's jars.
# Usage: run.sh <Class> [args...]   e.g. run.sh StdlibDump > ../../data/stdlib.tsv
# LIB: jar directory (default: the reference runner's qvto-3.11.0 set). OUT: class directory.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)  # the QVTo root, crates/qvto
LIB=${LIB:-$ROOT/reference/runner/lib/qvto-3.11.0}
[ -d "$LIB" ] || LIB=/home/devbox/workspace/qvto-rs/reference/runner/lib/qvto-3.11.0
OUT=${OUT:-${CARGO_TARGET_DIR:-$ROOT/../../target}/types-oracle}
CP=$(ls "$LIB"/*.jar | tr '\n' ':')
CLS=$1
shift
mkdir -p "$OUT"
if [ ! -f "$OUT/$CLS.class" ] || [ "$HERE/$CLS.java" -nt "$OUT/$CLS.class" ]; then
  nice -n 19 javac -nowarn -encoding UTF-8 -d "$OUT" -cp "$CP" "$HERE/$CLS.java"
fi
exec nice -n 19 java -Xmx2g -cp "$OUT:$CP" "$CLS" "$@"
