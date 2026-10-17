#!/bin/sh
# Compiles (if needed) and runs the Java oracle: requests on stdin, results on stdout.
#   QVTO_RUNNER_LIB  jar directory, default: <repo>/reference/runner/lib/qvto-3.11.0
#                    (falls back to the main checkout when run from a git worktree)
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)  # the QVTo root, crates/qvto
LIB=${QVTO_RUNNER_LIB:-$ROOT/reference/runner/lib/qvto-3.11.0}
if [ ! -d "$LIB" ]; then
  MAIN=$(cd "$(git -C "$ROOT" rev-parse --git-common-dir)/.." && pwd)/crates/qvto
  LIB=$MAIN/reference/runner/lib/qvto-3.11.0
fi
CP=$(ls "$LIB"/*.jar | tr '\n' ':')
OUT=${QVTO_ORACLE_BUILD:-${TMPDIR:-/tmp}/qvto-runtime-oracle}
mkdir -p "$OUT"
if [ ! -f "$OUT/Oracle.class" ] || [ "$HERE/Oracle.java" -nt "$OUT/Oracle.class" ] || [ "$HERE/OclEval.java" -nt "$OUT/Oracle.class" ]; then
  nice -n 19 javac -nowarn -d "$OUT" -cp "$CP" "$HERE/Oracle.java" "$HERE/OclEval.java"
fi
exec nice -n 19 java -Xmx2g -cp "$OUT:$CP" Oracle
