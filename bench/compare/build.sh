#!/bin/bash
# Builds the drivers of the simulator comparison (bench/compare/README in docs/performance/comparison.md).
#   SimOxide: bench/compare/simoxide-driver (own cargo workspace, release profile = SimOxide's)
#   Java:     javac only, against the product jars in place (refsim, stock SimuLizar 5.2.2, Slingshot)
set -e
C="$(cd "$(dirname "$0")" && pwd)"
source "$C/env.sh"
( cd "$C/simoxide-driver" && CARGO_TARGET_DIR="${CMP_CARGO_TARGET:-$R/target/compare}" cargo build --release -q )
( cd "$R" && cargo build --release -q -p simoxide-cli )
[ -f "$R/reference/build/classes-base/refsim/Main.class" ] || "$R/reference/build.sh"
"$R/reference/tools/classpath.sh" > /dev/null
rm -rf "$B"; mkdir -p "$B"/{refsim,simulizar,slingshot}
J="javac --release 17 -nowarn -proc:none -encoding UTF-8"
# refsim (patched): the driver lives in package refsim (package-private access to Measurements)
$J -d "$B/refsim" -cp "$R/reference/build/classes-patches:$R/reference/build/classes-base:$(cat "$R/reference/build/classpath.txt")" \
  "$C/java/common/cmp/Harness.java" "$C/java/refsim/refsim/CompareRefsim.java"
# stock SimuLizar: OSGi-ordered flat classpath of refsim (fixes commons-math order, docs/reference-simulator/patches.md "Classpath")
$J -d "$B/simulizar" -cp "$SL_CP" "$C/java/common/cmp/Harness.java" "$C/java/simulizar/cmpsl/CompareSimuLizar.java"
# Slingshot: sl-install bundle jars + nested jars + the work-slingshot headless bench jar (SlingshotRunner, PlainMain)
$J -d "$B/slingshot" -cp "$SS_CP" "$C/java/common/cmp/Harness.java" "$C/java/slingshot/cmpss/"*.java
echo "$B/slingshot:$SS_CP" | tr ':' '\n' > "$B/slingshot-classpath.txt"
echo "built drivers in $B"
