#!/bin/sh
# Builds the headless Eclipse QVTo runner (javac only). LIB selects the jar set.
set -e
cd "$(dirname "$0")"
LIB=${LIB:-lib/qvto-3.11.0}
CP=$(ls "$LIB"/*.jar | tr '\n' ':')
rm -rf build/classes && mkdir -p build/classes
nice -n 19 javac -nowarn -d build/classes -cp "$CP" $(find src -name '*.java')
printf '%s' "$CP" > build/classpath.txt
echo "built against $LIB"
