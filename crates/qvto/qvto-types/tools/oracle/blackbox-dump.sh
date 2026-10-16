#!/bin/sh
# Regenerates ../../data/blackboxes.tsv with BlackboxDump.java: the QVTo-visible signatures of the
# Java blackboxes of the corpus, loaded by Eclipse QVTo in the reference runner's environment.
# Needs the runner (reference/runner/build.sh, built here if missing) and the compile oracle's
# extra classes (crates/qvto-sema/tools/oracle/make-compile-fixture.sh builds them; built here
# if missing). Classes go to $CARGO_TARGET_DIR/types-oracle-bbox (default target/).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)
RUNNER=$ROOT/reference/runner
[ -f "$RUNNER/build/classpath.txt" ] || "$RUNNER/build.sh"
CP=$(sed "s#\(^\|:\)lib/#\1$RUNNER/lib/#g" "$RUNNER/build/classpath.txt")
STUBS=$ROOT/target/qvto-sema-oracle/stubs
if [ ! -d "$STUBS" ]; then
	T=$ROOT/reference/eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/sources
	mkdir -p "$STUBS"
	nice -n 19 javac -nowarn -encoding UTF-8 -proc:none -d "$STUBS" -cp "$RUNNER/build/classes:$CP" \
		$(find "$ROOT/crates/qvto-sema/tools/oracle/stubs" "$T/bug578265/src" "$T/bug579914/src" -name '*.java')
fi
OUT=${CARGO_TARGET_DIR:-$ROOT/target}/types-oracle-bbox
mkdir -p "$OUT"
nice -n 19 javac -nowarn -encoding UTF-8 -proc:none -d "$OUT" -cp "$RUNNER/build/classes:$CP" "$HERE/BlackboxDump.java"
Q=$ROOT/reference/eclipse-qvto
T=$Q/tests/org.eclipse.m2m.tests.qvt.oml
nice -n 19 java -Xmx2g -Dqvtorun.testbundle="$T" -Dqvtorun.oclplugin="$Q/plugins/org.eclipse.m2m.qvt.oml.ocl" \
	-cp "$OUT:$RUNNER/build/classes:$CP:$T:$Q/plugins/org.eclipse.m2m.qvt.oml.samples:$STUBS" qvtorun.BlackboxDump "$ROOT" \
	"$ROOT/crates/qvto-testkit/cases/eclipse.cases" "$ROOT/crates/qvto-testkit/cases/corpus.cases" \
	"$ROOT/crates/qvto-sema/tests/data/compile.cases" > "$HERE/../../data/blackboxes.tsv"
wc -l "$HERE/../../data/blackboxes.tsv"
