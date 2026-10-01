#!/bin/bash
# Builds the headless.simucom OSGi bundle against the Palladio 5.2.2 product and creates its OSGi configuration area.
set -e
H="$(cd "$(dirname "$0")" && pwd)"
P=${PALLADIO:-/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2}
JDK=${JDK:-/home/devbox/workspace/palladio-research/work-simucom/jre17}
JAVAC=${JAVAC:-javac}
W=$H/work; mkdir -p $W
cd $H/bundle
CP=$(ls $P/plugins/*.jar | tr '\n' ':')
rm -rf bin && mkdir bin
$JAVAC --release 17 -nowarn -cp "$CP" -d bin $(find src -name '*.java')
jar cfm $W/headless.simucom_1.0.0.jar META-INF/MANIFEST.MF -C bin . plugin.xml
rm -rf bin
# OSGi configuration area (like work-simucom/make-headless-config.sh, but with headless.simucom instead of headless.bench)
C=$W/config
rm -rf $C; mkdir -p $C/org.eclipse.equinox.simpleconfigurator
sed -e "s#^osgi.framework=file\\\\:plugins#osgi.framework=file\\\\:$P/plugins#" \
    -e 's/^eclipse.application=.*/eclipse.application=headless.simucom.app/' \
    -e "s#^osgi.bundles=reference\\\\:file\\\\:#osgi.bundles=reference\\\\:file\\\\:$P/plugins/#" \
    -e "s#^osgi.framework.extensions=reference\\\\:file\\\\:#osgi.framework.extensions=reference\\\\:file\\\\:$P/plugins/#" \
    -e '/eclipse.product/d;/splashPath/d;/p2/d' $P/configuration/config.ini > $C/config.ini
grep -vE '^org\.palladiosimulator\.(architecturaltemplates\.(jobs|ui)|simulizar\.action[a-z.]*),' $P/configuration/org.eclipse.equinox.simpleconfigurator/bundles.info \
  | sed "s#,plugins/#,file:$P/plugins/#" > $C/org.eclipse.equinox.simpleconfigurator/bundles.info
echo "headless.simucom,1.0.0,file:$W/headless.simucom_1.0.0.jar,4,false" >> $C/org.eclipse.equinox.simpleconfigurator/bundles.info
echo "built $W/headless.simucom_1.0.0.jar and $C"
