#!/bin/sh
# Regenerates src/java/chars_data.rs from the JDK's Character data.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${TMPDIR:-/tmp}/qvto-runtime-genchars
mkdir -p "$OUT"
nice -n 19 javac -nowarn -d "$OUT" "$HERE/GenChars.java"
nice -n 19 java -Xmx2g -cp "$OUT" GenChars > "$HERE/../../src/java/chars_data.rs"
wc -c "$HERE/../../src/java/chars_data.rs"
