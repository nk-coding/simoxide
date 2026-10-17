# Paths of the Slingshot reference runner (sourced by setup.sh, run.sh, smoke.sh).
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
B="$H/build"
# The headless Slingshot install (p2, ~130 MB). install.sh creates $H/install; older installs live in the
# palladio-research work directory, which moved from workspace/ to workspace/old/.
I=""
for d in "${SLINGSHOT_INSTALL:-}" "$H/install/sl-install" \
         /home/devbox/workspace/old/palladio-research/work-slingshot/sl-install \
         /home/devbox/workspace/palladio-research/work-slingshot/sl-install; do
  if [ -n "$d" ] && ls "$d"/plugins/org.palladiosimulator.analyzer.slingshot.core_*.jar >/dev/null 2>&1; then
    I="$(cd "$d" && pwd)"; break
  fi
done
JAVA=${JAVA:-java}
