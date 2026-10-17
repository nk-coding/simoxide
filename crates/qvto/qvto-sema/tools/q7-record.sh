#!/bin/sh
# Records Eclipse's compile problems for tests/data/q7/*.qvto (regressions found by the Q7
# fuzzing campaigns and the external corpus) next to each file as <name>.diag, with the
# reference runner (one JVM). Run from the QVTo root (crates/qvto).
set -e
D=qvto-sema/tests/data/q7
OUT=$(mktemp -d)
for f in $D/*.qvto; do
	n=$(basename "$f" .qvto)
	printf 'case q7_%s\ndriver q7\nkind compile\ndir %s\nqvto %s.qvto\nimports folder\ncheck problems 0 -1\nend\n\n' "$n" "$D" "$n"
done > "$OUT/cases"
reference/runner/qvto-run --batch "$OUT/cases" --out "$OUT" --base . --no-tape > /dev/null
for f in $D/*.qvto; do
	n=$(basename "$f" .qvto)
	grep '^problem' "$OUT/q7_$n/diag.txt" > "$D/$n.diag" || true
done
rm -rf "$OUT"
