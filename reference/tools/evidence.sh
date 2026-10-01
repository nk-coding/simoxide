#!/bin/bash
# Evidence for docs/reference-simulator/patches.md: compares measurements of the fully patched reference with
#   u1/u2: stock classes (REFSIM_UNPATCHED=1), two fresh JVMs
#   g1: without the trace/RNG/id patches (P1,P4,P5,P8), g2: without P2 (PS order), g3: without P3 (drain order)
# and checks trace stability without P2 (repeat 3 in one JVM).
cd "$(dirname "$0")/.."
T=$(mktemp -d); C=../corpus
G1=SimuComDefaultRandomNumberGenerator,AbstractSimProcessDelegator,SimuComSimProcess,EventDispatcher,StackContext,TransitionDeterminer,RDSeffSwitch,RDSeffPerformanceSwitch,UsageScenarioSwitch,AbstractScheduledResource,SimSimpleFairPassiveResource,ClosedWorkloadUser,OpenWorkload,AbstractExperiment
./refsim batch $C --out $T/pa --no-trace > $T/pa.log 2>&1 &
REFSIM_UNPATCHED=1 ./refsim batch $C --out $T/u1 --no-trace > $T/u1.log 2>&1 &
REFSIM_UNPATCHED=1 ./refsim batch $C --out $T/u2 --no-trace > $T/u2.log 2>&1 &
REFSIM_EXCLUDE=$G1 ./refsim batch $C --out $T/g1 --no-trace > $T/g1.log 2>&1 &
REFSIM_EXCLUDE=SimProcessorSharingResource ./refsim batch $C --out $T/g2 --no-trace > $T/g2.log 2>&1 &
REFSIM_EXCLUDE=ResourceTableManager ./refsim batch $C --out $T/g3 --no-trace > $T/g3.log 2>&1 &
REFSIM_EXCLUDE=SimProcessorSharingResource ./refsim batch $C --repeat 3 > $T/g2r.log 2>&1 &
wait
for g in pa u1 u2 g1 g2 g3; do echo "$g: $(tail -n 1 $T/$g.log)"; done
n=$(ls $T/pa | wc -l); echo "models: $n"
for g in u1 u2 g1 g2 g3; do
  echo -n "$g differs from patched: "; for d in $T/pa/*; do m=$(basename $d); cmp -s $d/measurements.csv $T/$g/$m/measurements.csv || echo -n "$m "; done; echo
done
echo -n "u1 vs u2 (stock, two JVMs) differ: "; for d in $T/u1/*; do m=$(basename $d); cmp -s $d/measurements.csv $T/u2/$m/measurements.csv || echo -n "$m "; done; echo
echo "without P2, trace not reproducible within one JVM (repeat 3):"; grep NONDET $T/g2r.log | awk '{print "  " $1}'
rm -rf $T
