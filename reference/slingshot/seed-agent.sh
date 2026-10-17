#!/bin/bash
# Builds the seed agent (the shared one, reference/oraclekit/seed-agent) for this runner: build/seed-agent.jar
# with build/asm.jar (the Slingshot install's ASM) next to it, and prints the jar's path. setup.sh calls it.
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
[ -n "$I" ] || { echo "no Slingshot install found (ASM comes from it); set SLINGSHOT_INSTALL" >&2; exit 1; }
exec "$H/../oraclekit/seed-agent.sh" "$B" "$(ls "$I"/plugins/org.objectweb.asm_9*.jar | head -1)"
