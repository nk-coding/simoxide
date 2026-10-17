#!/bin/sh
# Runs the EMF oracle: ./run.sh <command> <args...> (see src/EmfOracle.java).
set -e
D=$(cd "$(dirname "$0")" && pwd)
[ -f "$D/build/classpath.txt" ] || "$D/build.sh" >&2
exec nice -n 19 java -Xmx2g -cp "$D/build/classes:$(cat "$D/build/classpath.txt")" EmfOracle "$@"
