#!/bin/bash
# Builds the seed agent (seed-agent/, a -javaagent shared by the three reference harnesses) and prints
# the jar's path:
#
#   oraclekit/seed-agent.sh [OUT_DIR [ASM_JAR]]
#
# OUT_DIR (default reference/oraclekit/build) receives seed-agent.jar and asm.jar, the ASM jar the agent
# needs next to it (its manifest's Boot-Class-Path). ASM_JAR (default $ASM_JAR, else the ASM 9 of the
# Slingshot install, else that of the Palladio 5.2.2 product) is copied there. Each harness builds its
# own copy with the ASM of the product it runs: refsim (reference/build.sh: Palladio 5.2.2, release 17),
# the Slingshot runner (slingshot/seed-agent.sh: the Slingshot install), the QVTo runner's det mode
# (simoxide-oraclekit `java::seed_agent`: this default).
# Env: JAVA_RELEASE (javac --release, default 21), JAVAC, JAR.
set -euo pipefail
K="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:-$K/build}"
ASM="${2:-${ASM_JAR:-}}"
if [ -z "$ASM" ]; then
  source "$K/../slingshot/env.sh"   # $I: the Slingshot install, if any
  for d in "$I/plugins" "$("$K/../tools/palladio-research.sh")/work-simucom/palladio-5.2.2/plugins"; do
    ASM=$(ls "$d"/org.objectweb.asm_9*.jar 2>/dev/null | sort | tail -1) || true
    [ -n "$ASM" ] && break
  done
fi
[ -n "$ASM" ] && [ -f "$ASM" ] || { echo "no ASM 9 jar (set ASM_JAR or SLINGSHOT_INSTALL)" >&2; exit 1; }
mkdir -p "$OUT"
C="$OUT/seed-agent-classes"; rm -rf "$C"; mkdir -p "$C"
cp "$ASM" "$OUT/asm.jar"
"${JAVAC:-javac}" -nowarn -XDignore.symbol.file --release "${JAVA_RELEASE:-21}" -d "$C" -cp "$OUT/asm.jar" \
  $(find "$K/seed-agent" -name '*.java' | sort) 2>&1 | { grep -v '^Note:' >&2 || true; }
"${JAR:-jar}" cfm "$OUT/seed-agent.jar" "$K/seed-agent/MANIFEST.MF" -C "$C" .
rm -rf "$C"
echo "$OUT/seed-agent.jar"
