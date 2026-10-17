#!/bin/sh
# Builds the EMF oracle of qvto-emf against the reference runner's jars (javac only).
set -e
cd "$(dirname "$0")"
LIB=$(cd "${LIB:-../../qvto/reference/runner/lib/qvto-3.11.0}" && pwd)
TESTS=../../qvto/reference/eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml/src
CP=$(ls "$LIB"/*.jar | tr '\n' ':')
rm -rf build && mkdir -p build/classes
# the Eclipse suite's model comparer (TreeComparator), compiled from the reference checkout
CMP=$(find "$TESTS/org/eclipse/m2m/tests/qvt/oml/api/framework/comparator" -name '*.java' 2>/dev/null || true)
nice -n 19 javac -nowarn -d build/classes -cp "$CP" $(find src -name '*.java') $CMP
printf '%s' "$CP" > build/classpath.txt
echo "built against $LIB"
