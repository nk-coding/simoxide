#!/bin/bash
# Determinism check for the whole corpus:
#  1. two batch runs in two fresh JVMs must produce byte-identical outputs,
#  2. within one JVM every model is run 3x (--repeat 3) and must be identical each time,
#  3. the outputs must equal the stored corpus/*/expected files.
set -e
cd "$(dirname "$0")"
T=$(mktemp -d)
./refsim batch ../corpus --out "$T/a" --repeat 3 > "$T/a.log"; tail -1 "$T/a.log"
./refsim batch ../corpus --out "$T/b" > "$T/b.log"; tail -1 "$T/b.log"
if diff -rq "$T/a" "$T/b"; then echo "fresh JVMs: identical"; else echo "fresh JVMs: DIFFERENT"; exit 1; fi
./refsim batch ../corpus --check > "$T/c.log" || { grep -v ' ok$' "$T/c.log"; exit 1; }
tail -1 "$T/c.log"; echo "expected/: identical"
rm -rf "$T"
