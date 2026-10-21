#!/bin/bash
# Benchmark B1 of DESIGN-SPD.md (§6 "Performance comparison plan"): SimOxide's Slingshot flavour and
# Slingshot-exact mode against stock Slingshot (the S0 reference runner), SPD workloads in SimOxide's
# Slingshot and SimuLizar flavours against Slingshot, snapshot and fork costs, SimuLizar's
# reconfiguration rules against refsim. Results: bench/compare/results/<date>-b1/ (summary.md by
# summarize-b1.py); raw logs in bench/compare/spd/work/raw (git-ignored).
#
#   bench/compare/spd/b1.sh [build] [prep] [plain] [spd] [spdq] [snap] [rules] [summary]   (default: all; resumes)
#
# Env: REPS (3 processes per configuration, seeds 1..REPS), OUT (results dir), SIMS_PLAIN, SIMS_SPD.
set -u
H="$(cd "$(dirname "$0")" && pwd)"
source "$H/../lib.sh"
W="$H/work"
RAW="$W/raw"
OUT=${OUT:-$C/results/$(date +%F)-b1}
REPS=${REPS:-3}
SLINGSHOT="$R/reference/slingshot/run.sh"
SPD_BENCH="$R/target/release/examples/spd-bench"
CLI="$R/target/release/simoxide"
JVM=${JVM:--Xmx4g}
PHASES=${*:-build prep plain spd spdq snap rules summary}
mkdir -p "$RAW" "$OUT"

# plain (non-SPD) models: Slingshot flavour against Slingshot; short and long as in the main comparison
PLAIN_MODELS="x_ss_minimal x_espresso x_ss_mediastore"
# SPD models (reference/slingshot/models) and their simulated time
declare -A SPD_T=([spd_minimal]=100 [spd_time_policies]=100 [spd_scale_in]=150 [spd_service_group]=100
                  [spd_same_instant]=100 [spd_v4_cpu]=600 [spd_v4_rt]=600)
SPD_MODELS="spd_minimal spd_time_policies spd_scale_in spd_service_group spd_same_instant spd_v4_cpu spd_v4_rt"
spd_src() { echo "$R/reference/slingshot/models/${1%_long}"; }
spd_dir() { # flavour model: SimuLizar copies get time-driven utilisation monitors (SimuLizar needs a window)
  case $1 in simulizar) echo "$W/models-simulizar/${2%_long}" ;; *) spd_src "$2" ;; esac
}

done_log() { [ -f "$1" ] && grep -aq "^TIME .*rc=0" "$1"; }
measure() { # log 'command' -- one benchmark process, alone on a quiet machine
  local log=$1; shift
  done_log "$log" && return 0
  rm -f "$log" "$log.err"
  wait_quiet
  echo "$(date +%T) $(basename "$log")" >&2
  echo "# CMD $1" >> "$log"
  echo "# LOAD $(cut -d' ' -f1-3 /proc/loadavg)" >> "$log"
  python3 "$C/tools/timeit.py" "$log" "$1"
}

for p in $PHASES; do
  echo "=== b1 phase $p ($(date +%T))" >&2
  case $p in
    build)
      ( cd "$C/simoxide-driver" && CARGO_TARGET_DIR="${CMP_CARGO_TARGET:-$R/target/compare}" cargo build --release -q ) &&
      ( cd "$R" && cargo build --release -q -p simoxide-cli --features "${SIMOXIDE_FEATURES:-exact,hotspot-math,reconfiguration}" &&
        cargo build --release -q -p simoxide-reconf --example spd-bench ) || exit 1 ;;
    prep)
      for m in spd_minimal spd_time_policies spd_scale_in spd_service_group spd_same_instant spd_v4_cpu spd_v4_rt; do
        d="$W/models-simulizar/$m"; rm -rf "$d"; mkdir -p "$d"; cp -L "$(spd_src $m)"/* "$d"/
        python3 -I "$H/simulizar-monitors.py" "$d"/*.monitorrepository
      done ;;
    plain)
      for m in $PLAIN_MODELS; do for len in short long; do
        T=$(simtime "$m" "$len")
        if [ $len = short ]; then w=20 n=100 sr=31; else w=3 n=20 sr=8; fi
        for rep in $(seq 1 "$REPS"); do
          for v in slingshot:fast slingshot-exact:exact simulizar:exact simulizar:fast; do
            f=${v%%:*} md=${v##*:}
            measure "$RAW/plain__simoxide-${f}-${md}__${m}__${len}__$rep.log" \
              "$SIMOXIDE_DRV model=$R/corpus/$m simTime=$T maxMeas=-1 seed=$rep warmup=$w runs=$n flavour=$f simMode=$md"
            measure "$RAW/cold__simoxide-${f}-${md}__${m}__${len}__$rep.log" \
              "$CLI run --model $R/corpus/$m --seed $rep --max-sim-time $T --max-measurements -1 --flavour $f --mode $md --measurements $W/cold.csv"
          done
          # Slingshot (stock nightly, seeded, no instrumentation): run 0 is the JVM's first (cold), then warm
          measure "$RAW/plain__slingshot__${m}__${len}__$rep.log" \
            "env JVM_OPTS=$JVM NICE=0 $SLINGSHOT --stock -model $R/corpus/$m -simTime $T -seed $rep -instrument false -measurements false -runs $sr -log ERROR -out $W/ss/plain-$m-$len-$rep"
        done
        # sanity: one Slingshot run with measurements, compared with SimOxide's Slingshot flavour
        measure "$RAW/check__slingshot__${m}__${len}.log" \
          "env JVM_OPTS=$JVM NICE=0 $SLINGSHOT --stock -model $R/corpus/$m -simTime $T -seed 1 -instrument false -measurements true -log ERROR -out $W/ss/check-$m-$len"
      done; done ;;
    spd)
      for m in $SPD_MODELS; do
        T=${SPD_T[$m]}
        if [ "$T" -le 150 ]; then w=5 n=30 sr=31; else w=5 n=30 sr=8; fi
        for rep in $(seq 1 "$REPS"); do
          for v in slingshot:fast simulizar:fast simulizar:exact; do
            f=${v%%:*} md=${v##*:}
            measure "$RAW/spd__simoxide-${f}-${md}__${m}__$rep.log" \
              "$SPD_BENCH $(spd_dir "$f" "$m") flavour=$f mode=$md simTime=$T seed=$rep warmup=$w runs=$n"
          done
          for v in slingshot:fast slingshot-exact:exact simulizar:exact; do
            f=${v%%:*} md=${v##*:}
            measure "$RAW/spdcold__simoxide-${f}-${md}__${m}__$rep.log" \
              "$CLI run --model $(spd_dir "$f" "$m") --seed $rep --max-sim-time $T --max-measurements -1 --flavour $f --mode $md --spd-log $W/spd.jsonl --measurements $W/cold.csv"
          done
          measure "$RAW/spd__slingshot__${m}__$rep.log" \
            "env JVM_OPTS=$JVM NICE=0 $SLINGSHOT --stock -model $(spd_src "$m") -simTime $T -seed $rep -instrument false -measurements false -runs $sr -log ERROR -out $W/ss/spd-$m-$rep"
        done
        # sanity: Slingshot's decisions (SPD log) and measurements, one instrumented run
        measure "$RAW/check__slingshot__${m}.log" \
          "env JVM_OPTS=$JVM NICE=0 $SLINGSHOT --stock -model $(spd_src "$m") -simTime $T -seed 1 -trace false -tape false -measurements true -log ERROR -out $W/ss/check-$m"
      done ;;
    spdq)
      # the Slingshot flavour with stock Slingshot's quirks (as reference/slingshot/spd-stat.sh): same decisions
      for m in $SPD_MODELS; do
        T=${SPD_T[$m]}
        for rep in $(seq 1 "$REPS"); do
          measure "$RAW/spd__simoxide-slingshotq-fast__${m}__$rep.log" \
            "$SPD_BENCH $(spd_src "$m") flavour=slingshot mode=fast quirks=fast spdQuirks=slingshot simTime=$T seed=$rep warmup=5 runs=30"
        done
      done ;;
    snap)
      # fork, snapshot, restore and continuation of SPD runs (halfway); plain runs: simoxide-bench fork
      for m in $SPD_MODELS; do
        T=${SPD_T[$m]}; half=$((T / 2))
        for v in slingshot:fast simulizar:exact; do
          f=${v%%:*} md=${v##*:}
          measure "$RAW/snap__simoxide-${f}-${md}__${m}.log" \
            "$SPD_BENCH $(spd_dir "$f" "$m") flavour=$f mode=$md simTime=$T seed=1 warmup=2 runs=10 snapshotAt=$half"
        done
      done
      for md in exact fast; do
        measure "$RAW/fork__simoxide-bench__$md.log" "$R/target/release/simoxide-bench fork --reps 5 --mode $md"
      done
      # snapshot files through the CLI: pause halfway, write, resume (wall of each process)
      for m in x_ss_mediastore x_sl_mediastore; do
        T=$(simtime "$m" long); half=$((T / 2))
        for rep in $(seq 1 "$REPS"); do
          measure "$RAW/snapfile__pause__${m}__$rep.log" \
            "$CLI run --model $R/corpus/$m --seed 1 --max-sim-time $T --max-measurements -1 --mode exact --pause-at $half --snapshot $W/$m.snap"
          measure "$RAW/snapfile__resume__${m}__$rep.log" "$CLI resume $W/$m.snap --model $R/corpus/$m"
          measure "$RAW/snapfile__full__${m}__$rep.log" \
            "$CLI run --model $R/corpus/$m --seed 1 --max-sim-time $T --max-measurements -1 --mode exact"
        done
      done ;;
    rules)
      for m in lb_outsource lb_rules_qvto power_rules_qvto; do
        for rep in $(seq 1 "$REPS"); do
          for md in exact fast; do
            measure "$RAW/rules__simoxide-${md}__${m}__$rep.log" "$CLI run --model $R/corpus-rules/$m --seed $rep --mode $md"
          done
          measure "$RAW/rules__simoxide-exact-compat__${m}__$rep.log" "$CLI run --model $R/corpus-rules/$m --seed $rep --mode exact --qvto-mode compat"
          # refsim (patched SimuLizar 5.2.2 with rules, P9): seeds 1..11 in one JVM, the first is cold
          measure "$RAW/rules__refsim__${m}__$rep.log" \
            "env JVM_OPTS=$JVM ORACLE_NICE=0 $R/reference/refsim run --run-json $R/corpus-rules/$m/run.json --seeds 1:11"
        done
      done ;;
    summary) python3 -I "$H/summarize-b1.py" "$RAW" "$OUT" ;;
    *) echo "unknown phase $p" >&2; exit 1 ;;
  esac
done
