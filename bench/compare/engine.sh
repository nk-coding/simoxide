#!/bin/bash
# Re-measures SimOxide's own numbers (docs/performance/engine.md) and the cost figures of the guide and
# docs/qvto/performance.md on a quiet machine: the benchmark suite (exact, fast, Slingshot flavour),
# thread scaling, loading, fork and change costs, the reconfiguration host's costs, the HPA scaling-step
# cost and the QVTo engine's benchmark against Eclipse QVTo. Raw output in OUT/raw (git-ignored).
#
#   bench/compare/engine.sh [suite] [slingshot] [batch] [load] [fork] [change] [host] [hpa] [qvto]
#
# Env: OUT (results/<date>-engine), CORE (the core the single-threaded runs are pinned to, default 3)
set -u
C="$(cd "$(dirname "$0")" && pwd)"
R="$(cd "$C/../.." && pwd)"
source "$C/lib.sh"
OUT=${OUT:-$C/results/$(date +%F)-engine}
CORE=${CORE:-3}
B="$R/target/release/simoxide-bench"
PHASES=${*:-suite slingshot batch load fork change host hpa qvto}
mkdir -p "$OUT/raw"
newest() { ls -t "$R"/target/release/deps/$1-* 2>/dev/null | grep -v '\.d$' | head -1; }

for p in $PHASES; do
  wait_quiet
  echo "=== engine phase $p ($(date +%T), load $(cut -d' ' -f1-3 /proc/loadavg))" | tee -a "$OUT/raw/phases.log" >&2
  case $p in
    suite) for r in 1 2 3; do for md in exact fast; do
             taskset -c "$CORE" "$B" suite --reps 3 --mode $md > "$OUT/raw/suite-$md-$r.txt" 2>&1; done; done ;;
    slingshot) for r in 1 2 3; do for md in exact fast; do
             taskset -c "$CORE" "$B" suite --reps 3 --mode $md --flavour slingshot > "$OUT/raw/suite-slingshot-$md-$r.txt" 2>&1; done; done ;;
    batch) for md in exact fast; do
             "$B" batch --runs 176 --threads 1,2,4,8,16,22,25 --mode $md > "$OUT/raw/batch-$md.txt" 2>&1; done ;;
    load) for r in 1 2 3; do
             taskset -c "$CORE" "$R/target/release/examples/load-bench" bench --reps 300 \
               "$R/corpus/x_sl_mediastore" "$R/corpus/x_pem_screencast_ms" "$R/corpus/h01_ps_single" \
               "$R/crates/simoxide-cli/bench/models/gen_s6" > "$OUT/raw/load-$r.txt" 2>&1
             taskset -c "$CORE" "$R/target/release/examples/load-bench" bench --reps 300 --cache \
               "$R/corpus/x_sl_mediastore" "$R/corpus/x_pem_screencast_ms" "$R/corpus/h01_ps_single" \
               "$R/crates/simoxide-cli/bench/models/gen_s6" > "$OUT/raw/load-cache-$r.txt" 2>&1
             taskset -c "$CORE" "$R/target/release/examples/load-bench" bench --reps 300 --memory \
               "$R/corpus/x_sl_mediastore" "$R/corpus/x_pem_screencast_ms" "$R/corpus/h01_ps_single" \
               "$R/crates/simoxide-cli/bench/models/gen_s6" > "$OUT/raw/load-memory-$r.txt" 2>&1
           done ;;
    fork) for md in exact fast; do taskset -c "$CORE" "$B" fork --reps 5 --mode $md > "$OUT/raw/fork-$md.txt" 2>&1; done ;;
    change) for md in exact fast; do taskset -c "$CORE" "$B" change --reps 5 --mode $md > "$OUT/raw/change-$md.txt" 2>&1; done ;;
    host) for r in 1 2 3; do taskset -c "$CORE" "$(newest host)" costs --exact --nocapture > "$OUT/raw/host-costs-$r.txt" 2>&1; done ;;
    hpa) for r in 1 2 3; do
           ( cd "$R/crates/simoxide-reconf" && taskset -c "$CORE" "$(newest hpa)" cost_per_scaling_step --exact --ignored --nocapture ) \
             > "$OUT/raw/hpa-cost-$r.txt" 2>&1; done ;;
    qvto) # the Eclipse QVTo jars of the Java runner (crates/qvto/reference/runner/build.sh) moved with qvto-rs to old/
          q=/home/devbox/workspace/old/qvto-rs/reference/runner/lib/qvto-3.11.0
          [ -d "$R/crates/qvto/reference/runner/lib/qvto-3.11.0" ] || { [ -d "$q" ] && export LIB=${LIB:-$q}; }
          ( cd "$R" && QVTO_NICE=0 taskset -c 2,3 target/release-unwind/qvto bench --java --java-cold ) > "$OUT/raw/qvto-bench.txt" 2>&1
          rm -rf "$OUT/raw/qvto-bench"; cp -r "$R/crates/qvto/target/qvto-bench" "$OUT/raw/qvto-bench"
          cp "$OUT/raw/qvto-bench/last.md" "$OUT/qvto-bench.md" ;;
    *) echo "unknown phase $p" >&2; exit 1 ;;
  esac
done
python3 -I "$C/summarize-engine.py" "$OUT"
