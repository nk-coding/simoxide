#!/bin/sh
# Extracts the PCM 5.2.2 metamodels and default models from the Palladio product jars into
# <out>/plugins and writes <out>/pcm.conf and <out>/pcm-models.txt (the SimOxide corpus models
# and the default models) for tests/pcm_models.rs and the EMF oracle.
# Also the SimuLizar and SPD metamodels (from the source repositories) and the Slingshot/SPD
# example models.
# Usage: tools/pcm-setup.sh <out> [plugins-dir] [corpus-dir] [repos-dir]   (from the repository root)
set -e
OUT=$(mkdir -p "$1" && cd "$1" && pwd)
P=${2:-/home/devbox/workspace/old/palladio-research/work-simucom/palladio-5.2.2/plugins}
CORPUS=${3:-/home/devbox/workspace/simoxide/corpus}
REPOS=${4:-/home/devbox/workspace/old/palladio-research/repos}
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
echo "alias platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore http://www.eclipse.org/emf/2002/Ecore" >> "$CONF"
for d in "$REPOS"/Palladio-Addons-SPD-Metamodel/bundles/* "$REPOS"/Palladio-Analyzer-SimuLizar/bundles/*; do
  [ -d "$d/model" ] || continue
  echo "map platform:/plugin/$(basename "$d")/ $d/" >> "$CONF"
  echo "mmroot $d/model" >> "$CONF"
done
EX1="$REPOS/Palladio-Documentation-Slingshot/examples"
EX2="$REPOS/Palladio-Analyzer-Slingshot-E2E-Tests/bundles"
{
  find "$OUT/plugins/org.palladiosimulator.pcm.resources/defaultModels" "$OUT/plugins/org.palladiosimulator.metricspec.resources" -type f \
    \( -name '*.repository' -o -name '*.resourcetype' -o -name '*.metricspec' \)
  find "$CORPUS" -type f \( -name '*.repository' -o -name '*.system' -o -name '*.allocation' -o -name '*.resourceenvironment' \
    -o -name '*.usagemodel' -o -name '*.monitorrepository' -o -name '*.measuringpoint' \)
  find "$EX1" "$EX2" -type f -not -path '*/target/*' -not -path '*/bin/*' \( -name '*.repository' -o -name '*.system' \
    -o -name '*.allocation' -o -name '*.resourceenvironment' -o -name '*.usagemodel' -o -name '*.monitorrepository' \
    -o -name '*.measuringpoint' -o -name '*.spd' -o -name '*.semanticspd' -o -name '*.slo' -o -name '*.resourcetype' \)
  find "$REPOS"/Palladio-Addons-SPD-Metamodel/bundles/*/model "$REPOS"/Palladio-Analyzer-SimuLizar/bundles/*/model -name '*.ecore'
} | LC_ALL=C sort > "$OUT/pcm-models.txt"
wc -l "$OUT/pcm-models.txt"
