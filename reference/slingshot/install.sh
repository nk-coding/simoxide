#!/bin/bash
# Creates a headless Slingshot install (nightly) from the p2 update sites: the "fetch" step of the
# reference runner. Not needed when an install already exists (env.sh finds the one of 2026-09-29 in
# old/palladio-research/work-slingshot/sl-install, made by the same procedure).
#
#   ./install.sh            -> install/sl-install (~130 MB, gitignored), then ./setup.sh --lock to pin it
#
# Needs network, Java 21, and an Eclipse install with the p2 director and publisher bundles ($P, default:
# the Palladio 5.2.2 product in palladio-research/work-simucom). Slingshot has no releases: the update
# sites serve the current nightly, so a new install is generally NOT the pinned one (install.lock).
# Ported from old/palladio-research/work-slingshot/setup-slingshot.sh + install-slingshot.sh (2026-09).
# Not re-run in S0 (2026-10-17).
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PR=$("$H/../tools/palladio-research.sh")
W=${SLINGSHOT_WORK:-$H/install}
P=${P:-$PR/work-simucom/palladio-5.2.2}
REPOS_SRC=${REPOS_SRC:-$PR/repos}   # git clones: Palladio-Analyzer-Slingshot, Palladio-Addons-SPD-Metamodel
LAUNCHER=${LAUNCHER:-$(ls "$P"/plugins/org.eclipse.equinox.launcher_*.jar | head -1)}
mkdir -p "$W" && cd "$W"

REPOS=https://download.eclipse.org/releases/2025-12/202512101000/,\
https://download.eclipse.org/tools/orbit/simrel/orbit-aggregation/release/4.38.0/,\
https://download.eclipse.org/tools/orbit/downloads/2023-03/,\
https://updatesite.mdsd.tools/ecore-workflow/releases/1.2.0/,\
https://updatesite.palladio-simulator.com/palladio-build-updatesite/nightly/,\
https://updatesite.palladio-simulator.com/palladio-addons-spd-metamodel/nightly/,\
https://updatesite.palladio-simulator.com/palladio-analyzer-slingshot/nightly/,\
file:$W/localrepo

# 0. p2 director configuration borrowed from $P (read only)
mkdir -p director-config
cp -r "$P/configuration/config.ini" "$P/configuration/org.eclipse.equinox.simpleconfigurator" director-config/
sed -i '/eclipse.p2.data.area/d;/eclipse.p2.profile/d' director-config/config.ini
eclipse_app() { java -Dosgi.install.area=file:"$P"/ -jar "$LAUNCHER" -configuration "$W/director-config" -nosplash -consoleLog "$@"; }
director() { # IUs...
  eclipse_app -application org.eclipse.equinox.p2.director -repository "$REPOS" -installIU "$1" \
    -destination "$W/sl-install" -profile SlingshotHeadless -bundlepool "$W/sl-install" \
    -p2.os linux -p2.ws gtk -p2.arch x86_64 -roaming
}
publish() {
  rm -f localrepo/content.xml localrepo/artifacts.xml
  eclipse_app -application org.eclipse.equinox.p2.publisher.FeaturesAndBundlesPublisher \
    -metadataRepository file:"$W"/localrepo -artifactRepository file:"$W"/localrepo -source "$W"/localrepo -publishArtifacts
}
mkdir -p localrepo/plugins
[ -f localrepo/content.xml ] || publish

# 1. base install: Slingshot core feature + what common.utils needs to compile
director org.palladiosimulator.analyzer.slingshot.feature.feature.group,org.eclipse.equinox.simpleconfigurator,org.eclipse.equinox.launcher,org.eclipse.core.runtime,org.eclipse.equinox.common,org.apache.felix.scr,de.uka.ipd.sdq.simucomframework.variables,org.palladiosimulator.pcm,org.palladiosimulator.commons.stoex > install-1.log 2>&1
grep -E "Operation completed|failed" install-1.log

# 2. packaging bugs of the nightly: two required bundles are published nowhere
#    a) org.palladiosimulator.analyzer.slingshot.common.utils (Slingshot repo, missing from all features)
#    b) org.palladiosimulator.spd.semantic.transformations (QVTo files only, SPD-Metamodel repo)
mkdir -p nested build/common.utils/classes build/spdtrans
for j in sl-install/plugins/*.jar; do
  unzip -Z1 "$j" 2>/dev/null | grep -E '^(lib/)?[^/]*\.jar$' | while read -r n; do unzip -o -q -j "$j" "$n" -d nested/; done || true
done
CP=$(ls sl-install/plugins/*.jar nested/*.jar | tr '\n' ':')
SL_VER=$(ls sl-install/plugins/org.palladiosimulator.analyzer.slingshot.core_*.jar | sed 's/.*_\(.*\)\.jar/\1/')
SPD_VER=$(ls sl-install/plugins/org.palladiosimulator.spd_*.jar 2>/dev/null | sed 's/.*_\(.*\)\.jar/\1/' || echo 1.0.0.local)
SRC=$REPOS_SRC/Palladio-Analyzer-Slingshot/bundles/org.palladiosimulator.analyzer.slingshot.common.utils
javac -nowarn -d build/common.utils/classes -cp "$CP" $(find "$SRC/src" -name '*.java')
cp -r "$SRC/META-INF" build/common.utils/classes/
sed -i "s/1.0.0.qualifier/$SL_VER/" build/common.utils/classes/META-INF/MANIFEST.MF
(cd build/common.utils/classes && jar cfm "$W/localrepo/plugins/org.palladiosimulator.analyzer.slingshot.common.utils_$SL_VER.jar" META-INF/MANIFEST.MF .)
S=$REPOS_SRC/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations
cp -r "$S/META-INF" "$S/plugin.xml" "$S/transformations" build/spdtrans/
sed -i "s/1.0.0.qualifier/${SPD_VER:-1.0.0.local}/" build/spdtrans/META-INF/MANIFEST.MF
(cd build/spdtrans && jar cfm "$W/localrepo/plugins/org.palladiosimulator.spd.semantic.transformations_${SPD_VER:-1.0.0.local}.jar" META-INF/MANIFEST.MF .)
publish

# 3. the rest needed at run time:
#    SPD interpreter (pcm.core's NumberOfElementsMonitorBehavior needs a semanticspd Configuration binding),
#    recorderframework.edp2 (recorder looked up by name), pcm.resources (pathmap://PCM_MODELS)
director org.palladiosimulator.analyzer.slingshot.extension.pcm.core.feature.group,org.palladiosimulator.analyzer.slingshot.extension.monitoring.feature.feature.group,org.palladiosimulator.analyzer.slingshot.extension.spd.interpreter.feature.group,org.palladiosimulator.spd.semantic.transformations,org.palladiosimulator.recorderframework.edp2,org.palladiosimulator.pcm.resources > install-2.log 2>&1
grep -E "Operation completed|failed" install-2.log

# 4. Equinox configuration of the install (bundle list with start levels); the runner's setup.sh derives
#    its private configurations from configuration/org.eclipse.equinox.simpleconfigurator/bundles.info
python3 "$H/tools/install-bundles-info.py" sl-install
echo "done: SLINGSHOT_INSTALL=$W/sl-install ./setup.sh --lock && ./setup.sh"
