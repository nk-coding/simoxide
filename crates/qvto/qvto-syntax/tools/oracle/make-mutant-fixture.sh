#!/bin/sh
# Regenerates tests/fixtures/eclipse-mutants-oracle.tsv: Eclipse's verdict on every corpus
# mutant (tests/common/mutate.rs). One line per mutant:
#   id  errors  warnings  first-error-offset  first-error-length  first-error-message
# (first = smallest offset; offset -1 without errors).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
CRATE=$(cd "$HERE/../.." && pwd)
TMP=${TMPDIR:-/tmp}/qvto-mutants.$$
mkdir -p "$TMP"
(cd "$CRATE" && nice -n 19 cargo run -q -p qvto-syntax --example mutants -- "$TMP/m") > "$TMP/index.tsv"
cut -f1 "$TMP/index.tsv" | "$HERE/run.sh" > "$TMP/oracle.tsv"
python3 -I - "$TMP/index.tsv" "$TMP/oracle.tsv" > "$CRATE/tests/fixtures/eclipse-mutants-oracle.tsv" <<'PY'
import sys
ids = dict(l.rstrip('\n').split('\t') for l in open(sys.argv[1], encoding='utf-8'))
res = {}
for l in open(sys.argv[2], encoding='utf-8'):
    f = l.rstrip('\n').split('\t')
    r = res.setdefault(f[0], [0, 0, None])
    if f[1] == 'OK':
        continue
    if f[1] == 'E':
        r[0] += 1
        cand = (int(f[2]), int(f[3]), f[5])
        if r[2] is None or cand[:2] < r[2][:2]:
            r[2] = cand
    else:
        r[1] += 1
for path, mid in ids.items():
    e, w, first = res.get(path, [0, 0, None])
    off, ln, msg = first if first else (-1, 0, '')
    print(f"{mid}\t{e}\t{w}\t{off}\t{ln}\t{msg}")
PY
rm -rf "$TMP"
wc -l "$CRATE/tests/fixtures/eclipse-mutants-oracle.tsv"
