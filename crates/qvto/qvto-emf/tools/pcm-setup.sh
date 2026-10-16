#!/bin/sh
# Extracts the PCM 5.2.2 metamodels and default models from the Palladio product jars into
# <out>/plugins and writes <out>/pcm.conf and <out>/pcm-models.txt (the SimOxide corpus models
# and the default models) for tests/pcm_models.rs and the EMF oracle.
# Usage: tools/pcm-setup.sh <out> [plugins-dir] [corpus-dir]   (run from the repository root)
set -e
OUT=$(mkdir -p "$1" && cd "$1" && pwd)
P=${2:-/home/devbox/workspace/old/palladio-research/work-simucom/palladio-5.2.2/plugins}
CORPUS=${3:-/home/devbox/workspace/simoxide/corpus}
BUNDLES="de.uka.ipd.sdq.identifier de.uka.ipd.sdq.units de.uka.ipd.sdq.probfunction de.uka.ipd.sdq.stoex
org.palladiosimulator.pcm org.palladiosimulator.metricspec org.palladiosimulator.edp2
org.palladiosimulator.monitorrepository org.palladiosimulator.monitorrepository.map
org.palladiosimulator.pcm.edp2.measuringpoint org.palladiosimulator.simulizar.edp2.measuringpoint
org.palladiosimulator.pcm.resources org.palladiosimulator.metricspec.resources"
CONF="$OUT/pcm.conf"
: > "$CONF"
for b in $BUNDLES; do
  j=$(ls "$P"/"${b}"_[0-9]*.jar | tail -1)
  mkdir -p "$OUT/plugins/$b"
  unzip -o -q "$j" '*.ecore' 'defaultModels/*' 'models/*' '*.metricspec' -d "$OUT/plugins/$b" 2>/dev/null || true
  echo "map platform:/plugin/$b/ $OUT/plugins/$b/" >> "$CONF"
done
echo "map pathmap://PCM_MODELS/ $OUT/plugins/org.palladiosimulator.pcm.resources/defaultModels/" >> "$CONF"
echo "map pathmap://METRIC_SPEC_MODELS/ $OUT/plugins/org.palladiosimulator.metricspec.resources/" >> "$CONF"
echo "mmroot $OUT/plugins" >> "$CONF"
{
  find "$OUT/plugins/org.palladiosimulator.pcm.resources/defaultModels" "$OUT/plugins/org.palladiosimulator.metricspec.resources" -type f \
    \( -name '*.repository' -o -name '*.resourcetype' -o -name '*.metricspec' \)
  find "$CORPUS" -type f \( -name '*.repository' -o -name '*.system' -o -name '*.allocation' -o -name '*.resourceenvironment' \
    -o -name '*.usagemodel' -o -name '*.monitorrepository' -o -name '*.measuringpoint' \)
} | LC_ALL=C sort > "$OUT/pcm-models.txt"
wc -l "$OUT/pcm-models.txt"
