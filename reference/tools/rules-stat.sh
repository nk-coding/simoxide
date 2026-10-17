#!/bin/bash
# Statistical comparison of SimOxide with the patched SimuLizar reference on the reconfiguration
# examples (corpus-rules/, DESIGN-SPD R1): seeds FROM..TO of each case on both sides, then
# reference/slingshot/tools/compare_stat.py per case (Welch, Mann-Whitney, KS, Holm).
#   tools/rules-stat.sh OUT_DIR SIMOXIDE_BINARY [FROM TO] [case...]
# SIMOXIDE_BINARY: e.g. a default (EPL) build with `--features reconfiguration`, which runs the
# fast simulator mode and QVTo's fast mode. The reference runs one JVM at a time, niced.
# With EXACT_BIN (an exact build with `reconfiguration`): also counts the seeds whose measurements
# equal the reference's byte for byte (exact mode, QVTo fast mode), and a control that compares
# the reference's seeds with the exact build's next seeds (SimuLizar against itself: the
# methodology's false-rejection rate).
set -e
cd "$(dirname "$0")/.."
OUT=$(mkdir -p "$1" && cd "$1" && pwd); BIN=$2; FROM=${3:-1}; TO=${4:-30}; shift 4 || shift $#
CASES=${*:-lb_outsource lb_rules_qvto lb_scaleout power_rules_qvto sl_reconfiguration}
for c in $CASES; do
	d=../corpus-rules/$c
	mkdir -p "$OUT/ref/$c" "$OUT/sx/$c"
	if [ ! -f "$OUT/ref/$c/$TO.csv" ]; then
		nice -n 19 ./refsim run --run-json "$d/run.json" --seeds "$FROM:$TO" --measurements "$OUT/ref/$c/{seed}.csv" \
			> "$OUT/ref/$c.log" 2>&1
	fi
	for s in $(seq "$FROM" "$TO"); do
		nice -n 10 "$BIN" run --model "$d" --seed "$s" --measurements "$OUT/sx/$c/$s.csv" > /dev/null 2>> "$OUT/sx/$c.log"
	done
	echo "== $c"
	python3 slingshot/tools/compare_stat.py --csv "$OUT/$c.tsv" "$OUT"/ref/"$c"/*.csv -- "$OUT"/sx/"$c"/*.csv | tail -n 1
	if [ -n "$EXACT_BIN" ]; then
		mkdir -p "$OUT/exact/$c" "$OUT/control/$c"
		same=0
		for s in $(seq "$FROM" "$TO"); do
			nice -n 10 "$EXACT_BIN" run --model "$d" --mode exact --qvto-mode fast --seed "$s" \
				--measurements "$OUT/exact/$c/$s.csv" > /dev/null 2>&1
			cmp -s "$OUT/exact/$c/$s.csv" "$OUT/ref/$c/$s.csv" && same=$((same + 1))
		done
		echo "exact mode: $same of $((TO - FROM + 1)) seeds byte-identical to the reference"
		n=$((TO - FROM + 1))
		for s in $(seq $((TO + 1)) $((TO + n))); do
			nice -n 10 "$EXACT_BIN" run --model "$d" --mode exact --qvto-mode fast --seed "$s" \
				--measurements "$OUT/control/$c/$s.csv" > /dev/null 2>&1
		done
		echo -n "control (reference seeds $FROM-$TO vs $((TO + 1))-$((TO + n))): "
		python3 slingshot/tools/compare_stat.py --csv "$OUT/$c.control.tsv" "$OUT"/ref/"$c"/*.csv -- "$OUT"/control/"$c"/*.csv | tail -n 1
	fi
done
