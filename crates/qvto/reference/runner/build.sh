#!/bin/sh
# Builds the headless Eclipse QVTo runner (javac only). LIB selects the jar set.
#
# Besides the runner it compiles, from the uncommitted reference checkout, what the Eclipse test
# bundle provides at run time: the generated packages of the samples plugin (simpleuml, rdb) and
# of the test bundle (testqvt, generics), the test bundle's Java blackboxes (bbox/*,
# TestBlackboxLibrary, the Java source folders of parserTestData/sources listed in its
# build.properties, the bug577992 and bug507955 libraries that the suite compiles in its
# workspace, the buddy bundle's classes), its model comparer (TreeComparator), and qvto-emf's
# CanonDump/SuiteCompare.
set -e
cd "$(dirname "$0")"
R=$(pwd)
LIB=${LIB:-lib/qvto-3.11.0}
CP=$(ls "$LIB"/*.jar | tr '\n' ':')
Q=../eclipse-qvto
T=$Q/tests/org.eclipse.m2m.tests.qvt.oml
TS=$T/src/org/eclipse/m2m/tests/qvt/oml
ORACLE=../../../foundation/simoxide-emf/oracle/src
rm -rf build/classes && mkdir -p build/classes
SRC=$(find src -name '*.java')
EXTRA=""
if [ -d "$T" ]; then
	EXTRA="$(find $Q/plugins/org.eclipse.m2m.qvt.oml.samples/src_generated $T/src_generated -name '*.java' | grep -v -E '/(provider|presentation)/')
$(find $TS/bbox $TS/api/framework/comparator -name '*.java')
$TS/TestBlackboxLibrary.java
$TS/util/CustomEmfResourceFactory.java
$(find $T/parserTestData/models/bug577992/src -name '*.java')
$(sed -n 's#^.*\(parserTestData/sources/[^/]*/src\).*$#\1#p' $T/build.properties | grep -v bug577992 | while read d; do find $T/$d -name '*.java'; done)
$(find $Q/tests/org.eclipse.m2m.tests.qvt.oml.buddy/src -name '*.java')
$(find $Q/tests/org.eclipse.m2m.tests.qvto.transformationProject/transforms/models/bug507955/src -name '*.java')
$ORACLE/CanonDump.java $ORACLE/SuiteCompare.java"
else
	echo "warning: $T missing; building the runner without the test bundle's classes" >&2
fi
# The test bundle's AllTests.java needs JUnit; src-stubs has a stub with the bundle id.
nice -n 19 javac -nowarn -encoding UTF-8 -proc:none -d build/classes -cp "$CP" $SRC $(find src-stubs -name '*.java') $EXTRA
printf '%s' "$CP" > build/classpath.txt
echo "built against $LIB"
