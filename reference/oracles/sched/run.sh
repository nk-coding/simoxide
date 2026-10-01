#!/bin/bash
# Usage: run.sh patched|unpatched <script>   -> trace on stdout
# patched   = product jars + insertion-order PS patch (the semantics SimOxide reproduces)
# unpatched = product jars only (PS ties follow identity-hash order: nondeterministic)
set -e
D=$(cd "$(dirname "$0")" && pwd)
PRE=""
[ "$1" = patched ] && PRE="$D/build/patched:"
exec ${JAVA:-java} -Xmx256m -Dlog4j.configuration=file:/home/devbox/workspace/palladio-research/work-simucom/log4j-quiet.properties \
  -cp "${PRE}$D/build/classes:$(cat "$D/build/classpath.txt")" schedoracle.Main "$2"
