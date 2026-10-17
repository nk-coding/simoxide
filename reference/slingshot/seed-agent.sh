#!/bin/bash
# Builds the seed agent alone (build/seed-agent.jar with build/asm.jar next to it) and prints the jar's path.
# setup.sh calls it; the QVTo harness (crates/qvto, `qvto conform record --det`) calls it to run the Java
# QVTo runner with the det ordering rule: `-javaagent:build/seed-agent.jar=ordered,qvto` makes the
# HashSet/HashMap of QVTo's and OCL's classes LinkedHashSet/LinkedHashMap, as in `run.sh` (det mode).
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
JAVAC=${JAVAC:-javac}
JAR=${JAR:-jar}
[ -n "$I" ] || { echo "no Slingshot install found (ASM comes from it); set SLINGSHOT_INSTALL" >&2; exit 1; }
mkdir -p "$B/classes/seed-agent"
cp "$(ls "$I"/plugins/org.objectweb.asm_9*.jar | head -1)" "$B/asm.jar"
$JAVAC -nowarn -XDignore.symbol.file --release 21 -d "$B/classes/seed-agent" -cp "$B/asm.jar" \
  $(find "$H/src/seed-agent" -name '*.java') 2>&1 | grep -E 'error' >&2 || true
$JAR cfm "$B/seed-agent.jar" "$H/src/seed-agent/MANIFEST.MF" -C "$B/classes/seed-agent" .
echo "$B/seed-agent.jar"
