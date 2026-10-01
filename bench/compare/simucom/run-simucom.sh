#!/bin/bash
# Runs SimuCom (Palladio 5.2.2, patched headless workflow) N times sequentially in one JVM.
# Usage: run-simucom.sh <model-dir> <simTime> <runs> <seed> [extra key=value args, e.g. dump=true keep=true]
# Prints per run: PHASES ... and RESULT sim=simucom model=... run=i wall_ms=... sim_ms=... requests=... mean_rt=... sim_end=...
# Requires ./build.sh to have been run once (creates work/headless.simucom_1.0.0.jar + work/config).
set -e
H="$(cd "$(dirname "$0")" && pwd)"
[ $# -ge 4 ] || { echo "usage: $0 <model-dir> <simTime> <runs> <seed> [key=value...]" >&2; exit 2; }
MODEL=$(cd "$1" && pwd); SIMTIME=$2; RUNS=$3; SEED=$4; shift 4
P=${PALLADIO:-/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2}
JAVA=${JAVA:-/home/devbox/workspace/palladio-research/work-simucom/jre17/bin/java}  # JDT in 5.2.2 needs <= Java 17
W=$H/work
[ -f $W/headless.simucom_1.0.0.jar ] || $H/build.sh >&2
WS=$(mktemp -d $W/ws.XXXXXX)          # fresh Eclipse workspace per invocation (generated plug-in projects live here)
CFG=$(mktemp -d $W/cfg.XXXXXX); cp -r $W/config/. $CFG/   # private copy of the OSGi config area (Equinox writes caches there)
trap 'rm -rf "$WS" "$CFG"' EXIT
# SimuCom codegen limitation: if the System has the same entityName as a Repository, the generated system class
# (e.g. h13_passive_contention.h13_passive_contention) obscures the repository's Java package and compilation fails.
# In that case run on a copy of the model dir whose System entityName gets the suffix "_System" (name only).
SYSN=$(perl -0ne 'print $1 if /entityName="([^"]*)"/' "$MODEL"/*.system)
for r in "$MODEL"/*.repository; do
  if [ "$(perl -0ne 'print $1 if /entityName="([^"]*)"/' "$r")" = "$SYSN" ]; then
    mkdir -p $WS/models; cp -r "$MODEL" $WS/models/; MODEL=$WS/models/$(basename "$MODEL")
    perl -0pi -e 's/entityName="([^"]*)"/entityName="$1_System"/' "$MODEL"/*.system
    echo "[simucom] note: renamed System '$SYSN' -> '${SYSN}_System' in a temp copy (name clash with repository)" >&2
    break
  fi
done
unset DISPLAY
$JAVA ${JVM_OPTS:--Xmx4g} -Dlog4j.configuration=file:$H/log4j-quiet.properties \
  -jar $P/plugins/org.eclipse.equinox.launcher_1.6.400.v20210924-0641.jar \
  -install $P -configuration $CFG -data $WS -nosplash -consoleLog \
  -application headless.simucom.app model=$MODEL simTime=$SIMTIME runs=$RUNS seed=$SEED maxMeasurements=-1 "$@"
