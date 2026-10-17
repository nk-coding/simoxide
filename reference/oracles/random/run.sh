#!/bin/bash
# Usage: run.sh <mode> <args...>   (see src/randomoracle/Main.java); JAVA=... selects the JVM.
PRDIR="$("$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/reference/tools/palladio-research.sh")"  # palladio-research (moved to old/)
set -e
D=$(cd "$(dirname "$0")" && pwd)
exec ${JAVA:-java} -Xmx1g -Dlog4j.configuration=file:$PRDIR/work-simucom/log4j-quiet.properties \
  -cp "$D/build/classes:$(cat "$D/build/classpath.txt")" randomoracle.Main "$@"
