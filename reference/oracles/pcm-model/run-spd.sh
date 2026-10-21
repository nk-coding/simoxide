#!/bin/bash
# Regenerates the SPD goldens of crates/simoxide-model: ./run-spd.sh [list] [out dir]
# Like run.sh, plus the generated SPD packages from the Slingshot jars (org.palladiosimulator.spd,
# .spd.semantic, .spd.measuringpoint 1.0.0.202609010039; EPL-2.0, see LICENSES section 6).
# The list (spd-models.txt: name TAB directory relative to this one) names directories with
# .spd/.semanticspd files.
set -e
cd "$(dirname "$0")"
PRDIR="$(../../tools/palladio-research.sh)"
P=${PALLADIO_PLUGINS:-$PRDIR/work-simucom/palladio-5.2.2/plugins}
SPD=${SPD_PLUGINS:-$PRDIR/work-slingshot/sl-install/plugins}
[ -d build/classes ] || ./build.sh
CP=build/classes:$(cat build/classpath.txt)
for b in org.palladiosimulator.spd_1 org.palladiosimulator.spd.semantic_1 org.palladiosimulator.spd.measuringpoint_1; do
  CP=$CP:$(ls "$SPD"/$b*.jar | tail -1)
done
exec ${JAVA:-java} ${JVM_OPTS:--Xmx2g} -cp "$CP" SpdOracle "$P" "${1:-spd-models.txt}" "${2:-golden-spd}"
