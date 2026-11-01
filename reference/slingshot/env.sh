# Paths of the Slingshot reference runner (sourced by setup.sh, run.sh, smoke.sh).
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
B="$H/build"
# The headless Slingshot install (p2, ~130 MB). install.sh creates $H/install; the pinned one of 2026-09-29
# lives in the palladio-research work directory (reference/tools/palladio-research.sh).
I=""
for d in "${SLINGSHOT_INSTALL:-}" "$H/install/sl-install" \
         "$("$H/../tools/palladio-research.sh" 2>/dev/null)/work-slingshot/sl-install"; do
  if [ -n "$d" ] && ls "$d"/plugins/org.palladiosimulator.analyzer.slingshot.core_*.jar >/dev/null 2>&1; then
    I="$(cd "$d" && pwd)"; break
  fi
done
JAVA=${JAVA:-java}
# the harnesses' shared shell helpers (JVM launch, run limits, result comparison)
source "$H/../oraclekit/lib.sh"
