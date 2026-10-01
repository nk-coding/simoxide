#!/bin/bash
# Builds the random/distribution oracle against the Palladio 5.2.2 product jars (referenced in place).
# Classpath: reference/build/classpath.txt (tools/classpath.sh), OSGi-ordered: the Orbit
# org.apache.commons.math 2.1 jar precedes the old copy embedded in desmoj (docs/reference-simulator/patches.md "Classpath").
set -e
cd "$(dirname "$0")"
CP=$(cat "$(../../tools/classpath.sh)")
rm -rf build; mkdir -p build/classes
echo "$CP" > build/classpath.txt
${JAVAC:-javac} --release 17 -nowarn -encoding UTF-8 -cp "$CP" -d build/classes $(find src -name '*.java')
echo "randomoracle built"
