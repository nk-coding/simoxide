#!/bin/bash
# Regenerates golden/*.txt from scripts/*.txt with the patched reference (insertion-order PS
# ties) and checks, for every script, whether the unpatched product classes give the same trace.
set -e
cd "$(dirname "$0")"
[ -d build/classes ] || ./build.sh
mkdir -p golden
filter() { grep -E '^[A-Z#] ' || true; }
for s in scripts/*.txt; do
  n=$(basename "$s" .txt)
  ./run.sh patched "$s" 2>/dev/null | filter > "golden/$n.txt"
  if ./run.sh unpatched "$s" 2>/dev/null | filter | cmp -s - "golden/$n.txt"; then u=same; else u=DIFFERS; fi
  echo "$n: $(grep -c . "golden/$n.txt") lines, unpatched: $u"
done
