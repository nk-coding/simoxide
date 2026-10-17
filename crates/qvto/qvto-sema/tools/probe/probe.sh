#!/bin/sh
# Asks Eclipse QVTo (the reference runner's jars) about single files, for comparisons with the
# analyser and the VM on small programs:
#   probe.sh Problems a.qvto ...   compile problems (line, offset, length, message)
#   probe.sh AstTree a.qvto        the AST of the operations (classes, names, types, positions)
#   probe.sh AstCalls a.qvto       the referred operation of every operation call (overloads)
# Running a transformation: reference/runner/qvto-run --qvto a.qvto [--param in:x.xmi ...].
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)
R=$ROOT/reference/runner
[ -f "$R/build/classpath.txt" ] || "$R/build.sh"
CP="$R/build/classes:$(sed "s#\(^\|:\)lib/#\1$R/lib/#g" "$R/build/classpath.txt")"
OUT=$ROOT/target/qvto-sema-probe
if [ ! -f "$OUT/Problems.class" ] || [ -n "$(find "$HERE" -name '*.java' -newer "$OUT/Problems.class")" ]; then
	mkdir -p "$OUT"
	nice -n 19 javac -nowarn -proc:none -d "$OUT" -cp "$CP" "$HERE"/*.java 2>&1 | grep -v '^Note:' || true
fi
Q=$ROOT/reference/eclipse-qvto
T=$Q/tests/org.eclipse.m2m.tests.qvt.oml
MAIN=$1; shift
exec nice -n 19 java -Xmx2g -Dqvtorun.testbundle="$T" -cp "$OUT:$CP:$T:$Q/plugins/org.eclipse.m2m.qvt.oml.samples" "$MAIN" "$@"
