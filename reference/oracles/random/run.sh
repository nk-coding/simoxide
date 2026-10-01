#!/bin/bash
# Usage: run.sh <mode> <args...>   (see src/randomoracle/Main.java); JAVA=... selects the JVM.
set -e
D=$(cd "$(dirname "$0")" && pwd)
exec ${JAVA:-java} -Xmx1g -Dlog4j.configuration=file:/home/devbox/workspace/palladio-research/work-simucom/log4j-quiet.properties \
  -cp "$D/build/classes:$(cat "$D/build/classpath.txt")" randomoracle.Main "$@"
