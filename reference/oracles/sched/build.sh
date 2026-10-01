#!/bin/bash
# Builds the scheduler oracle against the Palladio 5.2.2 product jars (referenced in place).
#  build/classes   : the oracle driver
#  build/patched   : SimProcessorSharingResource with insertion-order ties (LinkedHashMap);
#                    put FIRST on the classpath to shadow the product class (run.sh patched)
# Classpath: reference/build/classpath.txt (tools/classpath.sh; OSGi-ordered, docs/reference-simulator/patches.md "Classpath").
set -e
cd "$(dirname "$0")"
CP=$(cat "$(../../tools/classpath.sh)")
rm -rf build; mkdir -p build/classes build/patched
echo "$CP" > build/classpath.txt
${JAVAC:-javac} --release 17 -nowarn -encoding UTF-8 -cp "$CP" -d build/patched $(find patched -name '*.java')
${JAVAC:-javac} --release 17 -nowarn -encoding UTF-8 -cp "$CP" -d build/classes $(find src -name '*.java')
echo "schedoracle built"
