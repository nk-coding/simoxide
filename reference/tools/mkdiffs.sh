#!/bin/bash
# Writes reference/patches/diffs/<Class>.diff: unified diff of every shadowed class against its
# releases/5.2.2 source (palladio-research/src-5.2.2, shallow clones of the release tags).
cd "$(dirname "$0")/.."
SRC522=${SRC522:-/home/devbox/workspace/palladio-research/src-5.2.2}
rm -rf patches/diffs; mkdir -p patches/diffs
for f in $(cd patches/src && find . -name '*.java' | sed 's|^\./||' | sort); do
  orig=$(find "$SRC522" -path "*/src*/$f" | grep -v /test | head -1)
  diff -u --strip-trailing-cr --label "5.2.2/${orig#$SRC522/}" --label "patched/$f" "$orig" "patches/src/$f" > "patches/diffs/$(basename "$f" .java).diff"
done
wc -l patches/diffs/*.diff | tail -1
