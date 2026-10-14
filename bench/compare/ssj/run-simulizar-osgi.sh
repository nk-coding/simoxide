#!/bin/bash
# Stock SimuLizar 5.2.2 in the product's OSGi runtime (palladio-research/work-simucom/headless-bundle, the
# headless.bench application that work-simucom/run-bench.sh starts), on the overlay install of ssj/setup.sh.
# SimuLizar takes the engine from the preference de.uka.ipd.sdq.simulation/simulationEngineField, set by the
# configuration's eclipse.pluginCustomization file (work/engine-<engine>.ini).
#   run-simulizar-osgi.sh desmoj|ssj models=DIR runs=N threads=T maxMeasurements=M simTime=S [quiet=true]
set -e
H="$(cd "$(dirname "$0")" && pwd)"
E=$1; shift
W=${SSJ_WORK:-$H/work}
OVL=$W/palladio-5.2.2-ssj
CFG=$W/simulizar-osgi-$E   # persistent like run-bench.sh's headless-config (Equinox caches the resolved bundles)
[ -f "$CFG/config.ini" ] || { echo "missing $CFG: run ssj/setup.sh" >&2; exit 2; }
mkdir -p "$W/eclipse-ws-$E"
exec ${JAVA:-java} ${JVM_OPTS:--Xmx2g} -Dlog4j.configuration=file:$W/log4j-quiet.properties \
  -jar $OVL/plugins/$(cd $OVL/plugins && ls org.eclipse.equinox.launcher_*.jar | head -1) \
  -install $OVL -configuration $CFG -data $W/eclipse-ws-$E -nosplash -consoleLog \
  -application headless.bench.app "$@"
