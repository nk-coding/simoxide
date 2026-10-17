#!/bin/bash
# Runs the Slingshot reference runner in a fresh JVM (Equinox, private configuration area).
#
#   ./run.sh [--stock] [--slot N] -model DIR -out DIR -simTime T [-seed N] [runner options]
#
#   --stock    the unpatched nightly (seeding only with -seed); default "det": patched jars (patches/)
#              plus the agent's collection-ordering rule, for deterministic same-time event order
#   --slot N   Equinox configuration area build/osgi/<mode>-N (default: the first free one; locked
#              with flock, so concurrent runs never share an area)
# A -seed adds the seed agent (-javaagent). Runner options: see src/runner/.../Runner.java or
# docs/reference-simulator/slingshot.md. Env: JAVA, JVM_OPTS (default -Xmx2g), NICE or ORACLE_NICE (default 19).
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
mode=det slot=""
while [ $# -gt 0 ]; do
  case "$1" in
    --stock) mode=stock; shift ;;
    --det) mode=det; shift ;;
    --slot) slot=$2; shift 2 ;;
    *) break ;;
  esac
done
[ -f "$B/equinox/$mode/config.ini" ] || { echo "run ./setup.sh first" >&2; exit 1; }
# det: the agent always, with the collection-ordering rule; stock: the agent only to seed (-seed)
agent=()
[ $mode = det ] && agent=(-javaagent:"$B/seed-agent.jar=${ORDERING:-ordered,qvto}")
for a in "$@"; do [ "$a" = -seed ] && [ $mode = stock ] && agent=(-javaagent:"$B/seed-agent.jar"); done
launcher=$(ls "$I"/plugins/org.eclipse.equinox.launcher_*.jar | head -1)

mkdir -p "$B/osgi"
# lock a configuration area (fd 9 stays open, so the lock lives as long as the JVM)
d=""
for k in ${slot:-0 1 2 3 4 5 6 7}; do
  c="$B/osgi/$mode-$k"; mkdir -p "$c"; exec 9>"$c.lock"
  if [ -n "$slot" ]; then flock 9; d=$c; break; fi
  if flock -n 9; then d=$c; break; fi
done
[ -n "$d" ] || { echo "no free configuration area" >&2; exit 1; }
cp "$B/equinox/$mode/config.ini" "$d/config.ini"
ORACLE_NICE=${NICE:-${ORACLE_NICE:-19}}
oraclekit_exec_java ${JVM_OPTS:--Xmx2g} "${agent[@]}" \
  -Dosgi.install.area=file:"$I"/ -Dosgi.configuration.area=file:"$d"/ \
  -Dosgi.instance.area=file:"$d"/ws/ -Dosgi.user.area=file:"$d"/user/ \
  -Dlog4j.configuration=file:"$B"/log4j.properties \
  -jar "$launcher" -nosplash -consoleLog -application simoxide.slingshot.runner.app "$@"
