#!/bin/bash
# Compiles the StoEx oracles against the SimuLizar 5.2.2 product jars (flat classpath
# reference/build/classpath.txt, OSGi-ordered: tools/classpath.sh, docs/reference-simulator/patches.md "Classpath").
# Output: build/classes (not versioned).
set -e
cd "$(dirname "$0")"
CP_FILE=${CP_FILE:-$(../../tools/classpath.sh)}
mkdir -p build/classes
javac --release 17 -nowarn -cp "$(cat "$CP_FILE")" -d build/classes src/oracle/*.java
echo "built oracle classes in $(pwd)/build/classes"
