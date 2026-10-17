#!/bin/bash
# Records what SimuLizar's rules see in the runtime measurement model (the patched reference,
# reference/refsim, with rules/dump.qvto) on the reconfiguration test models of simoxide-sim:
# <model>.expected holds the rule's log lines, one block per run of the reconfiguration process.
# Also checks that the reference's outputs with the rule equal the models' expected outputs (the
# rule always succeeds, so a run with it equals a run without rules). Needs PALLADIO_PLUGINS.
set -e
cd "$(dirname "$0")"
R=../../../../reference
M=../../../simoxide-sim/tests/models
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
for d in "$M"/t_reconf_*/; do
	n=$(basename "$d")
	REFSIM_VERBOSE=true REFSIM_LOG=INFO nice -n 19 "$R/refsim" run --run-json "$d/run.json" --rules rules \
		--measurements "$T/m.csv" --trace "$T/t.jsonl" --tape "$T/u.jsonl" > "$T/log" 2>&1 || true
	grep -E '^[0-9:,]+ +INFO QVTOExecutor:[0-9]+ - ' "$T/log" | sed -E 's/^[0-9:,]+ +INFO QVTOExecutor:[0-9]+ - //' > "$n.expected"
	ok=same
	for p in measurements.csv:m.csv trace.jsonl:t.jsonl tape.jsonl:u.jsonl; do
		e="$d/expected/${p%%:*}"; o="$T/${p##*:}"
		if [ -f "$e.gz" ]; then zcat "$e.gz" | cmp -s - "$o" || ok=DIFFERENT; else cmp -s "$e" "$o" || ok=DIFFERENT; fi
	done
	echo "$n: $(grep -c '^reconfiguration' "$n.expected") runs, outputs $ok"
done
