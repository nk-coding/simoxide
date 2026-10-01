# Workload definitions and command builders of the simulator comparison (sourced by run-all.sh).
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"

# model -> "short long" simulated time (s); the same for every simulator
declare -A SIMTIME=(
  [x_ss_minimal]="1000 100000"
  [x_espresso]="100 2000"
  [x_ss_mediastore]="200000 10000000"
  [x_sl_mediastore]="1000000 25000000"
  [h13_passive_contention]="100 5000"
  [x_pem_fork]="100 5000"
)
ALL_MODELS="x_ss_minimal x_espresso x_ss_mediastore x_sl_mediastore h13_passive_contention x_pem_fork"
ALL_SIMS="simoxide refsim simulizar simulizar-vt slingshot simucom eventsim"
SEED=1
JOPTS=${JOPTS:--Xmx4g}                        # single-process JVM runs (default GC = G1)
JOPTS_MP=${JOPTS_MP:--Xmx1g -XX:+UseSerialGC} # one of N concurrent worker JVMs

# simulated time of model $1 for length $2 (short | long | a number of seconds)
simtime() {
  [[ $2 =~ ^[0-9]+$ ]] && { echo "$2"; return; }
  local t=(${SIMTIME[$1]}); [ "$2" = short ] && echo ${t[0]} || echo ${t[1]}
}

# Model directory per simulator. Slingshot needs an explicit demand ("1") on Acquire/ReleaseAction
# (bench/compare/models/*_ss, tools/add-acquire-demand.py); EventSim rejects such a demand. SimOxide and
# refsim give byte-identical measurements for both variants (run-all.sh check).
model_dir() { # sim model
  case "$1:$2" in
    slingshot*:h13_passive_contention) echo "$C/models/h13_passive_ss" ;;
    slingshot*:x_sl_mediastore) echo "$C/models/x_sl_mediastore_ss" ;;
    *) echo "$R/corpus/$2" ;;
  esac
}

# Does the simulator run the model? (known failures documented in docs/performance/comparison.md)
supported() { # sim model
  case "$1:$2" in
    slingshot*:x_pem_fork) return 1 ;;   # OutOfMemoryError / NoSuchElementException in ForkBehaviorContextHolder
    *) return 0 ;;
  esac
}

# Command line of a benchmark process: sim model len warmup runs [key=value ...]
# (threads=T, barrier=DIR, mode=batch, tag=X are passed through to the drivers)
cmd() {
  local sim=$1 model=$2 len=$3 w=$4 n=$5; shift 5
  local dir; dir=$(model_dir "$sim" "$model")
  local T; T=$(simtime "$model" "$len")
  local common="model=$dir simTime=$T maxMeas=-1 seed=$SEED warmup=$w runs=$n"
  local jo=${JO:-$JOPTS}
  case "$sim" in
    simoxide)     echo "$SIMOXIDE_DRV $common $*" ;;
    refsim)       echo "$JAVA $jo -cp $REFSIM_CP refsim.CompareRefsim $PLUGINS $common $*" ;;
    simulizar)    echo "$JAVA $jo -cp $B/simulizar:$SL_CP cmpsl.CompareSimuLizar $PLUGINS $common $*" ;;
    simulizar-vt) echo "$JAVA $jo -Dpalladio.virtualThreads=true -cp $VT_JAR:$B/simulizar:$SL_CP cmpsl.CompareSimuLizar $PLUGINS $common sim=simulizar-vt $*" ;;
    slingshot)    echo "$JAVA $jo -cp $B/slingshot:$SS_CP cmpss.CompareSlingshot $common $*" ;;
    slingshot-iso) local k=$1; shift
                  echo "$JAVA ${JO:--Xmx16g} -cp $B/slingshot cmpss.IsoMain $k $B/slingshot-classpath.txt $common sim=slingshot-iso $*" ;;
    simucom)      echo "env JVM_OPTS='$jo' $C/simucom/run-simucom.sh $dir $T $n $SEED warmup=$w $*" ;;
    eventsim)     echo "env JAVA_OPTS='$jo' $C/eventsim.sh $common $*" ;;
    # cold one-shot in the product's OSGi runtime (the way a Palladio user runs it headless)
    simulizar-osgi) echo "env JVM_OPTS='$jo' $WS/run-bench.sh models=$dir runs=1 threads=1 maxMeasurements=-1 simTime=$T quiet=true" ;;
    slingshot-osgi) echo "env JAVA_OPTS='$jo' $WSL/run-osgi.sh -model $dir -simTime $T -runs 1 -seed $SEED -log ERROR" ;;
    simoxide-cli) echo "$SIMOXIDE run --model $dir --seed $SEED --max-sim-time $T --max-measurements -1 --measurements $SCRATCH/simoxide-cold.csv" ;;
    *) echo "unknown sim $sim" >&2; return 1 ;;
  esac
}

SCRATCH=${SCRATCH:-$C/results/tmp}
mkdir -p "$SCRATCH"

# Waits until the machine is quiet: at most 2 runnable tasks on average over 1 s (sampled from
# /proc/loadavg, which counts this sampler itself), i.e. no other agent's job is running. At most 10 min.
wait_quiet() {
  local i=0
  while [ $i -lt 600 ]; do
    local r; r=$(for k in 1 2 3 4 5 6 7 8 9 10; do cut -d' ' -f4 /proc/loadavg | cut -d/ -f1; sleep 0.1; done | awk '{s+=$1} END {print s/NR}')
    awk -v r="$r" 'BEGIN{exit !(r <= 2.0)}' && return 0
    [ $i = 0 ] && echo "  (waiting: $r runnable tasks)" >&2
    i=$((i + 1))
  done
  echo "  WARNING: machine still busy ($r runnable)" >&2
}

# Memory floor of the watchdogs (kB): a benchmark is killed when MemAvailable drops below it.
export MEM_FLOOR_KB=${MEM_FLOOR_KB:-8000000}
mem_avail_kb() { awk '/MemAvailable/ {print $2}' /proc/meminfo; }

# Runs a command line (string) in its own process group with /usr/bin/time; appends TIME and the
# output to $log. Kills the group if MemAvailable drops below 8 GB. $1 = log, $2 = command.
run_logged() {
  local log=$1 c=$2
  echo "# CMD $c" >> "$log"
  echo "# LOAD $(cut -d' ' -f1-3 /proc/loadavg) MEMAVAIL_KB $(mem_avail_kb) START_EPOCH_MS $(date +%s%3N)" >> "$log"
  setsid python3 "$C/tools/timeit.py" "$log" "$c" &
  local pid=$!
  while kill -0 $pid 2>/dev/null; do
    if [ "$(mem_avail_kb)" -lt "$MEM_FLOOR_KB" ]; then
      echo "# KILLED: MemAvailable < $MEM_FLOOR_KB kB" >> "$log"; kill -9 -- -$pid 2>/dev/null
    fi
    sleep 0.2
  done
  wait $pid 2>/dev/null
}
