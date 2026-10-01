#!/bin/bash
# Builds the simoxide-model oracle: standalone EMF (no OSGi) with the generated PCM, monitor repository and
# measuring point packages of the Palladio 5.2.2 product (jars referenced in place; the shared flat
# classpath reference/build/classpath.txt, OSGi-ordered: tools/classpath.sh, docs/reference-simulator/patches.md "Classpath").
set -e
cd "$(dirname "$0")"
P=${PALLADIO_PLUGINS:-/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2/plugins}
JAVAC=${JAVAC:-javac}
mkdir -p build/classes
cp "$(../../tools/classpath.sh)" build/classpath.txt
$JAVAC --release 17 -nowarn -cp "$(cat build/classpath.txt)" -d build/classes src/*.java
echo "built simoxide-model oracle"
