#!/bin/bash
# Builds the deterministic SimuLizar 5.2.2 reference runner (refsim).
#  - build/classpath.txt : flat classpath of the Palladio 5.2.2 product jars (referenced in place),
#                          one jar per bundle symbolic name + nested Bundle-ClassPath jars (extracted to build/lib),
#                          ordered like the OSGi wiring (tools/classpath.sh; docs/reference-simulator/patches.md "Classpath")
#  - build/classes       : refsim + patched (shadowing) classes; this dir comes FIRST on the classpath
# Env: PALLADIO_PLUGINS (default: palladio-research/work-simucom/palladio-5.2.2/plugins), JAVAC
PRDIR="$("$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/reference/tools/palladio-research.sh")"  # palladio-research (moved to old/)
set -e
cd "$(dirname "$0")"
P=${PALLADIO_PLUGINS:-$PRDIR/work-simucom/palladio-5.2.2/plugins}
mkdir -p build
tools/classpath.sh > /dev/null
CP=$(cat build/classpath.txt)
J="${JAVAC:-javac} --release 17 -nowarn -encoding UTF-8"
# 1) refsim runner (no patches): build/classes-base
rm -rf build/classes-base build/classes-patches; mkdir -p build/classes-base build/classes-patches
find src -name '*.java' | sort > build/sources-base.txt
$J -cp "$CP" -d build/classes-base @build/sources-base.txt 2>&1 | grep -v '^Note:' || true
test -f build/classes-base/refsim/Main.class || { echo "BUILD FAILED (base)"; exit 1; }
# 2) patched (shadowing) classes: build/classes-patches, placed before the product jars
find patches -name '*.java' | sort > build/sources-patches.txt
if [ -s build/sources-patches.txt ]; then
  $J -cp "build/classes-base:$CP" -d build/classes-patches @build/sources-patches.txt 2>&1 | grep -v '^Note:' || true
  n=$(grep -c . build/sources-patches.txt); m=$(find build/classes-patches -name '*.class' | grep -v '\$' | wc -l)
  [ "$m" -ge "$n" ] || { echo "BUILD FAILED (patches: $m of $n top-level classes)"; exit 1; }
fi
# 3) patch P10: the det ordering rule's agent (reference/slingshot/src/seed-agent) with the product's ASM:
#    build/seed-agent.jar, build/asm.jar next to it (its Boot-Class-Path); refsim loads it with `ordered,qvto`
rm -rf build/classes-agent; mkdir -p build/classes-agent
cp "$(ls "$P"/org.objectweb.asm_9*.jar | sort | tail -1)" build/asm.jar
$J -cp build/asm.jar -d build/classes-agent $(find slingshot/src/seed-agent -name '*.java' | sort) 2>&1 | grep -v '^Note:' || true
(cd build && ${JAR:-jar} cfm seed-agent.jar ../slingshot/src/seed-agent/MANIFEST.MF -C classes-agent .)
test -f build/seed-agent.jar || { echo "BUILD FAILED (seed agent)"; exit 1; }
echo "refsim built: base $(find build/classes-base -name '*.class' | wc -l) classes, patches $(find build/classes-patches -name '*.class' | wc -l) classes, product classpath entries: $(tr ':' '\n' < build/classpath.txt | wc -l)"
