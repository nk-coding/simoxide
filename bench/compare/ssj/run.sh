#!/bin/bash
# The SSJ comparison in one command (bench/compare/SSJ.md): the website's benchmark matrix
# (docs/performance/comparison.md) for SimOxide and for SimuLizar 5.2.2 and SimuCom 5.2.2 on both of Palladio's
# simulation engines, DESMO-J and SSJ, measured in one session. Uses run-all.sh's phases unchanged.
#
#   bench/compare/ssj/run.sh [build] [cold] [warm] [sus1] [susN] [longrun] [par] [extra] [summary]
#                                     (default: all, in this order; every phase resumes, like run-all.sh)
# Env (besides run-all.sh's): OUT (ssj/results), SIMS ("simoxide simulizar simulizar-ssj simucom simucom-ssj"),
#   EXTRA_MODELS ("x_misarch"; phase extra = cold + warm of these), PAR_WORKERS (number of cores),
#   SSJ_LOCK (file that must not exist, default the MiSArch measurement lock; empty = no check)
set -u
source "$(cd "$(dirname "$0")" && pwd)/common.sh"
export OUT=${OUT:-$SSJ_HOME/results}
export SIMS=${SIMS:-"simoxide simulizar simulizar-ssj simucom simucom-ssj"}
export MODELS=${MODELS:-$ALL_MODELS}
export PAR_WORKERS=${PAR_WORKERS:-$(nproc)}
export SUSN_CONFIGS=${SUSN_CONFIGS:-"simoxide-threads:60 simoxide-batch:60 simulizar-threads:300 simulizar-procs:300 simulizar-ssj-threads:300 simulizar-ssj-procs:300"}
export CMP_COLUMNS=present   # tables: only the simulators of this session
EXTRA_MODELS=${EXTRA_MODELS-x_misarch}
LOCK=${SSJ_LOCK-/home/devbox/workspace/misarch-optimization/MEASURING.lock}
PHASES=${*:-build cold warm sus1 susN longrun par extra summary}
mkdir -p "$OUT"

# the website's numbers come from a 22-core, 34 GB machine; say so if this one differs
cores=$(nproc); memgb=$(awk '/MemTotal/ {printf "%d", $2 / 1024 / 1024}' /proc/meminfo)
if [ "$cores" -lt 22 ] || [ "$memgb" -lt 30 ]; then
  echo "NOTE: $cores cores, $memgb GB RAM (docs/performance: 22 cores, 34 GB). The all-core phases (par, susN) use" \
       "$PAR_WORKERS workers; JVM runs are killed below MEM_FLOOR_KB=${MEM_FLOOR_KB} kB available." >&2
fi

wait_lock() {
  [ -n "$LOCK" ] || return 0
  local i=0
  while [ -e "$LOCK" ]; do
    [ $i = 0 ] && echo "  (waiting for $LOCK to disappear)" >&2
    i=$((i + 1)); sleep 60
  done
}

for p in $PHASES; do
  wait_lock
  echo "=== ssj phase $p ($(date +%T))" >&2
  case $p in
    build)
      ( cd "$C/simoxide-driver" && CARGO_TARGET_DIR="${CMP_CARGO_TARGET:-$R/target/compare}" cargo build --release -q ) &&
      ( cd "$R" && cargo build --release -q -p simoxide-cli --features "${SIMOXIDE_FEATURES:-exact,hotspot-math}" ) &&
      "$SSJ_HOME/setup.sh" || { echo "build failed" >&2; exit 1; } ;;
    extra) [ -n "$EXTRA_MODELS" ] && MODELS="$EXTRA_MODELS" "$C/run-all.sh" cold warm ;;
    summary) "$C/run-all.sh" summary && python3 -I "$SSJ_HOME/docs-numbers.py" "$OUT" ;;
    *) "$C/run-all.sh" "$p" ;;
  esac
done
