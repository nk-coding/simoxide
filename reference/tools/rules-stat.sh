#!/bin/bash
# Statistical comparison of SimOxide with the patched SimuLizar reference on the reconfiguration
# examples (corpus-rules/, DESIGN-SPD R1): seeds FROM..TO of each case on both sides, then
# reference/slingshot/tools/compare_stat.py per case (Welch, Mann-Whitney, KS, Holm).
#   tools/rules-stat.sh OUT_DIR SIMOXIDE_BINARY [FROM TO] [case...]
# SIMOXIDE_BINARY: e.g. a default (EPL) build with `--features reconfiguration`, which runs the
# fast simulator mode and QVTo's fast mode. The reference runs one JVM at a time, niced.
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
	python3 slingshot/tools/compare_stat.py --csv "$OUT/$c.tsv" "$OUT"/ref/"$c"/*.csv -- "$OUT"/sx/"$c"/*.csv | tail -n 3
done
