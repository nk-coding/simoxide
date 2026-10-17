#!/bin/sh
# Regenerates simoxide-jcompat's src/chars_data.rs (crates/foundation) from the JDK's Character data.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${TMPDIR:-/tmp}/qvto-runtime-genchars
mkdir -p "$OUT"
nice -n 19 javac -nowarn -d "$OUT" "$HERE/GenChars.java"
nice -n 19 java -Xmx2g -cp "$OUT" GenChars > "$HERE/../../../../foundation/simoxide-jcompat/src/chars_data.rs"
wc -c "$HERE/../../../../foundation/simoxide-jcompat/src/chars_data.rs"
