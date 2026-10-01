#!/bin/bash
# Performance comparison of SimOxide against the other Palladio simulators (docs/performance/comparison.md).
#
#   bench/compare/run-all.sh [build] [check] [cold] [warm] [par] [summary]    (default: all, in this order)
#
# Env: SIMS, MODELS, LENS (short long), COLD_REPS (3), PAR_WORKERS (22), OUT (results dir),
#      MEM_FLOOR_KB (watchdog floor, default 8000000)
# Every benchmark process runs alone (the script waits for a quiet machine before each one) and is
# killed if MemAvailable drops below MEM_FLOOR_KB (default 8 GB). Raw driver output: $OUT/raw/<phase>/*.log.
set -u
C="$(cd "$(dirname "$0")" && pwd)"
source "$C/lib.sh"
OUT=${OUT:-$C/results}
SIMS=${SIMS:-$ALL_SIMS}
MODELS=${MODELS:-$ALL_MODELS}
LENS=${LENS:-short long}
COLD_REPS=${COLD_REPS:-3}
PAR_WORKERS=${PAR_WORKERS:-22}
PHASES=${*:-build check cold warm sus1 susN longrun summary}
mkdir -p "$OUT/raw"/{cold,warm,par,check}

has() { [[ " $1 " == *" $2 "* ]]; }

phase_build() { "$C/build.sh"; [ -f "$C/simucom/work/headless.simucom_1.0.0.jar" ] || "$C/simucom/build.sh"; }

# The *_ss model variants (explicit acquire demand for Slingshot) must not change SimuLizar's results.
phase_check() {
  local ok=0
  for m in h13_passive_contention x_sl_mediastore; do
    local v; v=$(model_dir slingshot $m); local T; T=$(simtime $m short)
    for d in "$R/corpus/$m" "$v"; do
      "$SIMOXIDE" run --model "$d" --seed $SEED --max-sim-time $T --max-measurements -1 \
        --measurements "$OUT/raw/check/simoxide_$(basename "$d").csv" 2>/dev/null
      "$R/reference/refsim" run --model "$d" --seed $SEED --max-sim-time $T --max-measurements -1 \
        --measurements "$OUT/raw/check/refsim_$(basename "$d").csv" 2>/dev/null
    done
    for s in simoxide refsim; do
      if cmp -s "$OUT/raw/check/${s}_$m.csv" "$OUT/raw/check/${s}_$(basename "$v").csv"; then
        echo "check: $s $m == $(basename "$v")"
      else echo "check: $s $m DIFFERS from $(basename "$v")"; ok=1; fi
    done
    cmp -s "$OUT/raw/check/simoxide_$m.csv" "$OUT/raw/check/refsim_$m.csv" && echo "check: simoxide == refsim ($m)" \
      || { echo "check: simoxide != refsim ($m)"; ok=1; }
  done
  return $ok
}

# Cold one-shot: one process per run (process start -> results), COLD_REPS repetitions.
phase_cold() {
  local sims=""
  for s in $SIMS; do
    case $s in
      simoxide) sims="$sims simoxide-cli" ;;
      simulizar) sims="$sims simulizar simulizar-osgi" ;;
      slingshot) sims="$sims slingshot slingshot-osgi" ;;
      *) sims="$sims $s" ;;
    esac
  done
  for m in $MODELS; do for len in $LENS; do for s in $sims; do
    supported "$s" "$m" || continue
    # OSGi variants: short runs of three models only (startup cost does not depend on the length)
    [[ $s == *-osgi ]] && { [ "$len" = short ] && has "x_ss_minimal x_espresso x_ss_mediastore" "$m" || continue; }
    for rep in $(seq 1 "$COLD_REPS"); do
      local log="$OUT/raw/cold/${s}__${m}__${len}__${rep}.log"
      [ -f "$log" ] && grep -aq "^TIME .*rc=0" "$log" && continue   # resume
      rm -f "$log" "$log.err"
      wait_quiet; echo "cold $s $m $len #$rep" >&2
      run_logged "$log" "$(cmd "$s" "$m" "$len" 0 1)"
    done
  done; done; done
}

# Warm: one long-lived process per (sim, model, length): warm-up runs, then measured runs.
phase_warm() {
  for m in $MODELS; do for len in $LENS; do for s in $SIMS; do
    supported "$s" "$m" || continue
    local w n
    case "$s:$len" in
      simoxide:short) w=50; n=200 ;;  simoxide:long) w=3; n=20 ;;
      simucom:short) w=5; n=10 ;;   *:short) w=20; n=20 ;;  *:long) w=2; n=10 ;;
    esac
    local log="$OUT/raw/warm/${s}__${m}__${len}.log"
    [ -f "$log" ] && grep -aq "^TIME .*rc=0" "$log" && continue   # resume: keep completed processes
    rm -f "$log" "$log.err"
    wait_quiet; echo "warm $s $m $len (w=$w n=$n)" >&2
    run_logged "$log" "$(cmd "$s" "$m" "$len" $w $n)"
  done; done; done
}

# N worker processes (or isolated instances) started together through a file barrier.
par_barrier() { # log k cmd...
  local log=$1 k=$2; shift 2
  local bar="$SCRATCH/barrier.$$"; rm -rf "$bar"; mkdir -p "$bar"
  echo "# CMD $* (x$k, barrier)" >> "$log"
  echo "# LOAD $(cut -d' ' -f1-3 /proc/loadavg) MEMAVAIL_KB $(mem_avail_kb)" >> "$log"
  "$@" "$bar" "$log" &
  local pid=$! t=0
  while [ "$(ls "$bar" | grep -c '^ready\.')" -lt "$k" ] && kill -0 $pid 2>/dev/null; do
    sleep 0.1; t=$((t + 1))
    [ "$(mem_avail_kb)" -lt "$MEM_FLOOR_KB" ] && { echo "# KILLED: MemAvailable < $MEM_FLOOR_KB kB" >> "$log"; pkill -9 -f "barrier=$bar"; }
    [ $t -gt 6000 ] && { echo "# TIMEOUT waiting for workers" >> "$log"; break; }
  done
  echo "# GO_EPOCH_MS $(date +%s%3N) MEMAVAIL_KB $(mem_avail_kb)" >> "$log"
  touch "$bar/go"
  local minmem=99999999999
  while kill -0 $pid 2>/dev/null; do
    local a; a=$(mem_avail_kb); [ "$a" -lt "$minmem" ] && minmem=$a
    [ "$a" -lt "$MEM_FLOOR_KB" ] && { echo "# KILLED: MemAvailable < $MEM_FLOOR_KB kB" >> "$log"; pkill -9 -f "barrier=$bar"; }
    sleep 0.2
  done
  wait $pid
  echo "# END_EPOCH_MS $(date +%s%3N) MIN_MEMAVAIL_KB $minmem" >> "$log"
  rm -rf "$bar"
}

# k processes of the same command, each with barrier=DIR (run in their own process group)
spawn_procs() { # sim model len w n k bar log
  local s=$1 m=$2 len=$3 w=$4 n=$5 k=$6 bar=$7 log=$8
  local c; c=$(JO="$JOPTS_MP" cmd "$s" "$m" "$len" "$w" "$n" "barrier=$bar")
  setsid bash -c "for i in \$(seq 1 $k); do $c > '$log.w'\$i 2>> '$log.err' & done; wait" &
  wait $!
  cat "$log".w* >> "$log"; rm -f "$log".w*
}
spawn_iso() { # model len w n k bar log
  local c; c=$(cmd slingshot-iso "$1" "$2" "$3" "$4" "$5" "barrier=$6")
  setsid bash -c "$c >> '$7' 2>> '$7.err'" &
  wait $!
}

# Parallel throughput with all cores, the way each tool can.
PAR_SET=${PAR_SET:-"x_espresso:short"}
phase_par() {
  local k=$PAR_WORKERS
  for ml in $PAR_SET; do
    local m=${ml%%:*} len=${ml##*:} w n
    [ "$len" = short ] && { w=10; n=20; } || { w=1; n=3; }
    has "$MODELS" "$m" || continue
    for s in $SIMS; do
      supported "$s" "$m" || continue
      local log
      case $s in
        simoxide)
          for mode in reload batch; do
            log="$OUT/raw/par/simoxide-$mode-t${k}__${m}__${len}.log"; rm -f "$log"
            wait_quiet; echo "par simoxide $mode $m $len" >&2
            local runs=$((k * 50)); [ "$len" = short ] && runs=$((k * 1000))
            run_logged "$log" "$(cmd simoxide "$m" "$len" 0 $runs threads=$k mode=$mode)"
          done ;;
        simulizar|simulizar-vt)   # in one JVM: sequential warm-up first (OCL/EMF lazy-init race), shared RNG
          log="$OUT/raw/par/$s-t${k}__${m}__${len}.log"; rm -f "$log"
          wait_quiet; echo "par $s threads $m $len" >&2
          JO="-Xmx16g" run_logged "$log" "$(JO=-Xmx16g cmd "$s" "$m" "$len" 1 $((k * n)) threads=$k)" ;;&
        slingshot)                # isolated class loaders in one JVM
          log="$OUT/raw/par/slingshot-iso-k${k}__${m}__${len}.log"; rm -f "$log" "$log.err"
          wait_quiet; echo "par slingshot iso $m $len" >&2
          par_barrier "$log" $k spawn_iso "$m" "$len" $w $n $k ;;&
        refsim|simulizar|slingshot|simucom)   # k worker processes
          local kk=$k; [ $s = simucom ] && kk=${SIMUCOM_WORKERS:-16}
          log="$OUT/raw/par/$s-p${kk}__${m}__${len}.log"; rm -f "$log" "$log.err"
          wait_quiet; echo "par $s x$kk processes $m $len" >&2
          par_barrier "$log" $kk spawn_procs "$s" "$m" "$len" $w $n $kk ;;
      esac
    done
  done
}

# ---------------------------------------------------------------- sustained performance
# timeit with process-tree sampling (RSS, threads every 2 s) and optional timeout
run_sampled() { # log timeout_s cmd
  local log=$1 to=$2 c=$3
  echo "# CMD $c" >> "$log"
  echo "# LOAD $(cut -d' ' -f1-3 /proc/loadavg) MEMAVAIL_KB $(mem_avail_kb) START_EPOCH_MS $(date +%s%3N)" >> "$log"
  python3 "$C/tools/timeit.py" --sample "$log.rss.csv" --interval 2 ${to:+--timeout $to} "$log" "$c"
}
gcopt() { echo "-Xlog:gc:file=$1.gc.%p.txt:uptimemillis"; }

# 1. Sustained single core: one process per simulator runs the model back to back (fresh seed per run)
#    for a fixed wall time, from JVM start (no warm-up, to see the JIT warm-up curve).
SUS1_SET=${SUS1_SET:-"x_ss_mediastore:2000000:300 x_espresso:100:120"}   # model:simTime:seconds
phase_sus1() {
  mkdir -p "$OUT/raw/sus1"
  for spec in $SUS1_SET; do
    IFS=: read -r m T D <<< "$spec"
    has "$MODELS" "$m" || continue
    for s in $SIMS; do
      supported "$s" "$m" || continue
      local d=$D; [ $s = simoxide ] && d=$(( D < 60 ? D : 60 ))
      local log="$OUT/raw/sus1/${s}__${m}__${T}.log"
      [ -f "$log" ] && grep -aq "^TIME .*rc=0" "$log" && continue
      rm -f "$log"*
      wait_quiet; echo "sus1 $s $m T=$T ${d}s" >&2
      # stock SimuLizar exactly as shipped: the RNG is never disposed (one leaked producer thread per run)
      local extra=""; [ $s = simulizar ] && extra="disposeRng=false"
      run_sampled "$log" $((d + 180)) "$(JO="$JOPTS $(gcopt "$log")" cmd "$s" "$m" "$T" 0 1 duration=$d varySeed=true $extra)"
    done
  done
}

# 2. Sustained all cores (22): aggregate throughput over time, memory, failures.
SUSN_SET=${SUSN_SET:-"x_ss_mediastore:2000000"}
SUSN_CONFIGS=${SUSN_CONFIGS:-"simoxide-threads:60 simoxide-batch:60 simulizar-threads:300 simulizar-procs:300 simulizar-vt-threads:180 slingshot-iso:300 slingshot-procs:300 refsim-procs:180"}
phase_susN() {
  mkdir -p "$OUT/raw/susN"
  local k=$PAR_WORKERS
  for spec in $SUSN_SET; do
    IFS=: read -r m T <<< "$spec"
    for cd in $SUSN_CONFIGS; do
      local cfg=${cd%%:*} D=${cd##*:}
      local s=${cfg%-*} how=${cfg##*-}
      has "$SIMS" "$s" || continue
      local log="$OUT/raw/susN/${cfg}-${k}__${m}__${T}.log"
      [ -f "$log" ] && grep -aq "^TIME .*rc=0" "$log" && continue
      rm -f "$log"*
      wait_quiet; echo "susN $cfg x$k $m T=$T ${D}s" >&2
      local sus="duration=$D varySeed=true"
      case $how in
        threads)
          case $s in
            simoxide) run_sampled "$log" $((D + 120)) "$(cmd simoxide "$m" "$T" 0 1 threads=$k $sus)" ;;
            *) # in one JVM: one sequential warm-up run first (OCL/EMF lazy-init race), per-thread EDP2 repositories
               local jo="-Xmx12g $(gcopt "$log")"
               run_sampled "$log" $((D + 240)) "$(JO="$jo" cmd "$s" "$m" "$T" 1 1 threads=$k $sus)" ;;
          esac ;;
        batch) run_sampled "$log" $((D + 120)) "$(cmd simoxide "$m" "$T" 0 1 threads=$k mode=batch $sus)" ;;
        iso|procs)
          local bar="$SCRATCH/barrier.sus.$$"; rm -rf "$bar"; mkdir -p "$bar"
          local sh="$SCRATCH/spawn.$$.sh" n
          if [ $how = iso ]; then
            n=1
            echo "$(JO="-Xmx16g $(gcopt "$log")" cmd slingshot-iso "$m" "$T" 0 1 $k barrier=$bar $sus) > '$log.w1' 2>> '$log.err'" > "$sh"
          else
            n=$k
            local one; one=$(JO="$JOPTS_MP $(gcopt "$log")" cmd "$s" "$m" "$T" 0 1 barrier=$bar $sus)
            { echo "for i in \$(seq 1 $k); do"; echo "  $one > '$log.w'\$i 2>> '$log.err' &"; echo "done; wait"; } > "$sh"
          fi
          run_sampled "$log" $((D + 400)) "bash $sh" &
          local pid=$! t=0
          while [ "$(ls "$bar" | grep -c '^ready\.')" -lt "$k" ] && kill -0 $pid 2>/dev/null && [ $t -lt 3000 ]; do
            sleep 0.1; t=$((t + 1)); done
          echo "# GO_EPOCH_MS $(date +%s%3N) READY $(ls "$bar" | grep -c '^ready\.') MEMAVAIL_KB $(mem_avail_kb)" >> "$log"
          touch "$bar/go"
          wait $pid
          cat "$log".w* >> "$log" 2>/dev/null; rm -f "$log".w* "$sh"; rm -rf "$bar" ;;
      esac
    done
  done
}

# 3. One very long simulation per simulator (x_ss_mediastore): simulated time up to 100x "long", chosen so
#    that the run should take about 4 min if its cost per simulated second stays constant (from the warm
#    "long" median); 300 s wall cap. RSS/threads sampled every 2 s; refsim also reports in-run progress.
LONG_MODEL=${LONG_MODEL:-x_ss_mediastore}
phase_longrun() {
  mkdir -p "$OUT/raw/longrun"
  local m=$LONG_MODEL base; base=$(simtime $m long)
  python3 "$C/summarize.py" "$OUT" > /dev/null
  for s in $SIMS; do
    supported "$s" "$m" || continue
    local ts
    if [ $s = simoxide ]; then ts="$((base * 100)) $((base * 500))"; else  # 1000x overflows i64 ns (292 years)
      ts=$(python3 -c "
import csv, sys
w = [float(r['warm_wall_ms']) for r in csv.DictReader(open('$OUT/summary.csv'))
     if r['sim'] == '$s' and r['model'] == '$m' and r['len'] == 'long' and r['warm_wall_ms']]
f = min(100, 240 / (w[0] / 1000)) if w else 10
f = max(1, int(f)) if f < 10 else int(f // 10 * 10)
print(int($base * f))")
    fi
    for T in $ts; do
      local log="$OUT/raw/longrun/${s}__${m}__${T}.log"
      [ -f "$log" ] && grep -aq "^TIME .*rc=0" "$log" && continue
      rm -f "$log"*
      wait_quiet; echo "longrun $s $m T=$T" >&2
      local jo="-Xmx12g $(gcopt "$log")"
      run_sampled "$log" 300 "$(JO="$jo" cmd "$s" "$m" "$T" 0 1 progress=5000)"
    done
  done
}

phase_summary() { python3 "$C/summarize.py" "$OUT"; python3 "$C/sustained.py" "$OUT"; }

for p in $PHASES; do
  echo "=== phase $p ($(date +%T))" >&2
  "phase_$p" || echo "phase $p had failures" >&2
done
