#!/bin/bash
# Regenerates the committed golden files (small, used by `cargo test -p simoxide-random`).
# Large dumps for the --ignored tests:  SCALE=big OUT=<dir> ./gen-golden.sh
set -e
D=$(cd "$(dirname "$0")" && pwd)
OUT=${OUT:-$D/golden}; mkdir -p "$OUT"
if [ "$SCALE" = big ]; then NU=1000000; NM=400000; ND=20000; NP=20000; else NU=100000; NM=4000; ND=300; NP=500; fi
"$D/run.sh" uniforms "$OUT/uniforms.txt" $NU 1,2,3,4,5,6
"$D/run.sh" uniforms "$OUT/uniforms-b.txt" 2000 -2147483648,2147483647,0,-1,123456789,42
"$D/run.sh" mathfns "$OUT/mathfns.txt" $NM 7
"$D/run.sh" dists "$OUT/dists.txt" $ND 1000
"$D/run.sh" probfn "$OUT/probfn.txt" $NP 2000
ls -la "$OUT"
