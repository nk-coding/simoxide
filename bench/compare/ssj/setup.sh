#!/bin/bash
# Sets up Palladio 5.2.2 on the SSJ engine next to DESMO-J (bench/compare/SSJ.md). Idempotent; writes only below
# bench/compare/ssj/work/ (git-ignored). The Palladio install in palladio-research is read, never modified.
#
#   1. downloads the SSJ engine bundle and the SSJ library of the Palladio 5.2.2 release (pinned SHA-256)
#   2. overlay install work/palladio-5.2.2-ssj: symlinks to every product plugin + the two SSJ bundles,
#      configuration/ = the product's plus two bundles.info lines
#   3. engine preference files work/engine-{desmoj,ssj}.ini (eclipse.pluginCustomization)
#   4. SimuCom: simucom/build.sh into work/simucom-{desmoj,ssj} (OSGi configs on the overlay, engine preference)
#   5. SimuLizar OSGi (headless.bench application): work/simulizar-osgi-{desmoj,ssj}
#   6. SimuLizar flat-classpath driver (java/simulizar, engine=desmoj|ssj) into work/build/simulizar
#   7. models: work/models/x_misarch_simucom (Java-safe names for SimuCom)
set -e
source "$(cd "$(dirname "$0")" && pwd)/common.sh"
P=$WS/palladio-5.2.2
[ -d "$P/plugins" ] || { echo "Palladio 5.2.2 product not found at $P (set PALLADIO_RESEARCH)" >&2; exit 1; }
mkdir -p "$SSJ_WORK/dl"

# ---------------------------------------------------------------- 1. download
# p2 repositories of the 5.2.2 release (the SHA-256 values are those of their artifacts.xml)
U_ASE=https://updatesite.palladio-simulator.com/palladio-simulation-abstractsimengine/releases/5.2.2
U_TPL=https://updatesite.palladio-simulator.com/palladio-thirdparty-library/releases/5.2.2
fetch() { # url sha256
  local f="$SSJ_WORK/dl/$(basename "$1")"
  if [ ! -f "$f" ] || ! echo "$2  $f" | sha256sum -c --quiet - 2> /dev/null; then
    curl -fsSL -m 300 -o "$f.part" "$1"
    echo "$2  $f.part" | sha256sum -c --quiet - || { echo "checksum mismatch: $1" >&2; rm -f "$f.part"; exit 1; }
    mv "$f.part" "$f"
  fi
}
fetch $U_ASE/plugins/org.palladiosimulator.simulation.abstractsimengine.ssj_5.2.2.jar \
  7385ee9fd789885e10475b185987403157a9610c09a715ee1c8cb2ac4e664831
fetch $U_TPL/plugins/ca.umontreal.iro.simul.ssj_3.3.1.jar \
  f6d882c55101f6a18846451da4dca6f6c085dbd50a25327a4c264c93cd8fa5f3
# the same release as the installed product: its DESMO-J engine bundle is byte-identical to the release's
echo "f7a5afcd445a3b68afc2ea64638f7109a574af7a0371f966fe2236dd3f5bf922  $P/plugins/de.uka.ipd.sdq.simulation.abstractsimengine.desmoj_5.2.2.jar" \
  | sha256sum -c --quiet - || echo "WARNING: the installed DESMO-J engine bundle is not the 5.2.2 release's" >&2

# ---------------------------------------------------------------- 2. overlay install
O=$SSJ_OVL
rm -rf "$O"; mkdir -p "$O/plugins" "$O/configuration/org.eclipse.equinox.simpleconfigurator"
for f in "$P"/plugins/*; do ln -s "$f" "$O/plugins/"; done
ln -s "$P/features" "$O/features"
for j in org.palladiosimulator.simulation.abstractsimengine.ssj_5.2.2.jar ca.umontreal.iro.simul.ssj_3.3.1.jar; do
  [ -e "$O/plugins/$j" ] && { echo "$j is already in the product: not an overlay" >&2; exit 1; }
  cp "$SSJ_WORK/dl/$j" "$O/plugins/$j"
done
cp "$P/configuration/config.ini" "$O/configuration/"
{ cat "$P/configuration/org.eclipse.equinox.simpleconfigurator/bundles.info"
  echo "ca.umontreal.iro.simul.ssj,3.3.1,plugins/ca.umontreal.iro.simul.ssj_3.3.1.jar,4,false"
  echo "org.palladiosimulator.simulation.abstractsimengine.ssj,5.2.2,plugins/org.palladiosimulator.simulation.abstractsimengine.ssj_5.2.2.jar,4,false"
} > "$O/configuration/org.eclipse.equinox.simpleconfigurator/bundles.info"

# ---------------------------------------------------------------- 3. engine preferences
for e in desmoj ssj; do
  id=SSJ_ENGINE_ID_$e
  printf '# Palladio simulation engine (preference page "Palladio Simulation"; SimulationPreferencesHelper)\nde.uka.ipd.sdq.simulation/simulationEngineField=%s\n' \
    "${!id}" > "$SSJ_WORK/engine-$e.ini"
done
cp "$WS/log4j-quiet.properties" "$SSJ_WORK/log4j-quiet.properties"

# ---------------------------------------------------------------- 4. SimuCom
SIMUCOM_WORK="$SSJ_WORK/simucom-desmoj" PALLADIO="$O" JDK="$WS/jre17" "$C/simucom/build.sh"
rm -rf "$SSJ_WORK/simucom-ssj"; mkdir -p "$SSJ_WORK/simucom-ssj"
cp "$SSJ_WORK/simucom-desmoj/headless.simucom_1.0.0.jar" "$SSJ_WORK/simucom-ssj/"
cp -r "$SSJ_WORK/simucom-desmoj/config" "$SSJ_WORK/simucom-ssj/config"
sed -i "s#$SSJ_WORK/simucom-desmoj/#$SSJ_WORK/simucom-ssj/#" "$SSJ_WORK/simucom-ssj/config/org.eclipse.equinox.simpleconfigurator/bundles.info"
for e in desmoj ssj; do
  echo "eclipse.pluginCustomization=$SSJ_WORK/engine-$e.ini" >> "$SSJ_WORK/simucom-$e/config/config.ini"
done

# ---------------------------------------------------------------- 5. SimuLizar in OSGi (as work-simucom/make-headless-config.sh)
cp "$WS/headless-bundle/headless.bench_1.0.0.jar" "$SSJ_WORK/"
for e in desmoj ssj; do
  D="$SSJ_WORK/simulizar-osgi-$e"
  rm -rf "$D" "$SSJ_WORK/eclipse-ws-$e"; mkdir -p "$D/org.eclipse.equinox.simpleconfigurator"
  sed -e "s#^osgi.framework=file\\\\:plugins#osgi.framework=file\\\\:$O/plugins#" \
      -e 's/^eclipse.application=.*/eclipse.application=headless.bench.app/' \
      -e "s#^osgi.bundles=reference\\\\:file\\\\:#osgi.bundles=reference\\\\:file\\\\:$O/plugins/#" \
      -e "s#^osgi.framework.extensions=reference\\\\:file\\\\:#osgi.framework.extensions=reference\\\\:file\\\\:$O/plugins/#" \
      -e '/eclipse.product/d;/splashPath/d;/p2/d' "$O/configuration/config.ini" > "$D/config.ini"
  echo "eclipse.pluginCustomization=$SSJ_WORK/engine-$e.ini" >> "$D/config.ini"
  grep -vE '^org\.palladiosimulator\.(architecturaltemplates\.(jobs|ui)|simulizar\.action[a-z.]*),' \
      "$O/configuration/org.eclipse.equinox.simpleconfigurator/bundles.info" \
    | sed "s#,plugins/#,file:$O/plugins/#" > "$D/org.eclipse.equinox.simpleconfigurator/bundles.info"
  echo "headless.bench,1.0.0,file:$SSJ_WORK/headless.bench_1.0.0.jar,4,false" >> "$D/org.eclipse.equinox.simpleconfigurator/bundles.info"
done

# ---------------------------------------------------------------- 6. SimuLizar driver (flat classpath)
[ -e "${SL_CP%%:*}" ] || { echo "SimuLizar classpath not found (reference/build/classpath.txt, reference/tools/classpath.sh)" >&2; exit 1; }
rm -rf "$SSJ_WORK/build/simulizar"; mkdir -p "$SSJ_WORK/build/simulizar"
javac --release 17 -nowarn -proc:none -encoding UTF-8 -d "$SSJ_WORK/build/simulizar" -cp "$SL_CP" \
  "$C/java/common/cmp/Harness.java" "$C/java/simulizar/cmpsl/CompareSimuLizar.java"

# ---------------------------------------------------------------- 7. models
mkdir -p "$SSJ_WORK/models"
python3 -I "$SSJ_HOME/sanitize-names.py" "$SSJ_HOME/models/x_misarch" "$SSJ_WORK/models/x_misarch_simucom"

echo "SSJ setup done in $SSJ_WORK (Palladio: $PR)"
