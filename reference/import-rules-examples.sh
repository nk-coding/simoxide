#!/bin/bash
# Imports SimuLizar's reconfiguration examples (QVTo rules) into corpus-rules/ (DESIGN-SPD R1):
# model files flat with relative hrefs and the example's own monitors (`refsim import
# --keep-monitors`), the rules in <case>/rules/, run.json with `reconfiguration_rules` and the stop
# conditions of the example's launch configuration. Sources: SimuLizar `releases/5.2.2` (EPL-2.0,
# palladio-research/src-5.2.2/Palladio-Analyzer-SimuLizar, commit 23dc438); scaleOut.qvto
# (lb_scaleout) and addBranch.qvto (lb_add_branch) are SimOxide's (rules-examples/). Usage: import-rules-examples.sh [outDir]
# Afterwards: CORPUS=../corpus-rules ./regen-expected.sh (expected outputs of the patched reference).
set -e
PRDIR="$("$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/reference/tools/palladio-research.sh")"
cd "$(dirname "$0")"
OUT=$(mkdir -p "${1:-../corpus-rules}" && cd "${1:-../corpus-rules}" && pwd)
SL=$PRDIR/src-5.2.2/Palladio-Analyzer-SimuLizar
EX=$SL/bundles/ExampleModels
LB=$EX/org.palladiosimulator.simulizar.examples.loadbalancer
PW=$EX/TransientEffects/org.palladiosimulator.simulizar.examples.loadbalancer.powerconsumption
TM=$SL/tests/org.palladiosimulator.simulizar.tests/testmodels
OWN=$(pwd)/rules-examples
LOG=$(mktemp)
trap 'rm -f "$LOG"' EXIT

runjson() { # seed max_measurements max_sim_time
	printf '{\n  "seed": %s,\n  "max_measurements": %s,\n  "max_sim_time": %s,\n  "simulate_linking_resources": false,\n  "simulate_throughput_of_linking_resources": true,\n  "reconfiguration_rules": "rules"\n}\n' "$1" "$2" "$3"
}

# case files run.json rule-files...
import() {
	local name=$1 files=$2 rj=$3; shift 3
	local d="$OUT/$name"
	rm -rf "$d"; mkdir -p "$d/rules"
	if ! ./refsim import "$files" "$d" --keep-monitors --run-json "$rj" >"$LOG" 2>&1; then
		echo "FAIL $name: $(grep -m1 'FAILED\|Exception' "$LOG" | cut -c1-200)"; return 1
	fi
	cp "$@" "$d/rules/"
	# informational absolute paths of the import directory -> relative
	sed -i "s|file:$d/||g" "$d"/*.measuringpoint
	echo "ok   $name"
}

# 1. The load balancer example as its launch configuration runs it: reconfigurations_active
#    (outsource: branch probabilities of the load balancer's SEFF), 1000 s.
import lb_outsource "$LB/overload.usagemodel,$LB/loadbalancer_on_iaas.allocation,$LB/monitors/monitoring.monitorrepository" \
	"$(runjson 1 100000 1000)" "$LB/reconfigurations_active/outsource.qvto"
# 2. The load balancer example with the QVTo folder reconfigurations_qvto: outsource, scaleDown,
#    scaleIn, scaleUp (branch probabilities and processing rates).
import lb_rules_qvto "$LB/overload.usagemodel,$LB/loadbalancer_on_iaas.allocation,$LB/monitors/monitoring.monitorrepository" \
	"$(runjson 1 100000 1000)" "$LB"/reconfigurations_qvto/*.qvto
# 3. The load balancer with outsource and SimOxide's scaleOut.qvto: a new resource container (with
#    resources, on the LAN), a new assembly context allocated to it, the connector rewired.
import lb_scaleout "$LB/overload.usagemodel,$LB/loadbalancer_on_iaas.allocation,$LB/monitors/monitoring.monitorrepository" \
	"$(runjson 1 100000 1000)" "$LB/reconfigurations_active/outsource.qvto" "$OWN/scaleOut.qvto"
# 3b. The load balancer with SimOxide's addBranch.qvto: a third branch added to the load balancer's
#     SEFF at the first reconfiguration (a structural behaviour change that routes load).
import lb_add_branch "$LB/overload.usagemodel,$LB/loadbalancer_on_iaas.allocation,$LB/monitors/monitoring.monitorrepository" \
	"$(runjson 1 100000 1000)" "$OWN/addBranch.qvto"
# 4. The power-consumption variant (TransientEffects) with its PCM-only rules (reconf_qvto: scaleDown,
#    scaleIn, scaleUp), 2000 s. Its power monitors need the Palladio power extension, which is not
#    part of the 5.2.2 product: they are removed; its resource-URI measuring points with
#    platform:/resource URIs (Eclipse workspace) become the PCM measuring points they denote.
import power_rules_qvto "$PW/overload.usagemodel,$PW/loadbalancer_on_iaas.allocation,$PW/monitors/monitor.monitorrepository" \
	"$(runjson 1 -1 2000)" "$PW/reconf_qvto/scaleDown.qvto" "$PW/reconf_qvto/scaleIn.qvto" "$PW/reconf_qvto/scaleUp.qvto"
python3 - "$OUT/power_rules_qvto" <<'EOF'
import re, sys
d = sys.argv[1]
mp = d + '/loadBalancerMeasuringpointRepo.measuringpoint'
s = open(mp).read()
P = 'platform:/resource/org.palladiosimulator.simulizar.examples.loadbalancer.powerconsumption/'
def repl(name, new):
    global s
    pat = re.compile(r'<measuringPoints xsi:type="org.palladiosimulator.edp2.models:ResourceURIMeasuringPoint" stringRepresentation="%s"[^>]*/>' % name, re.S)
    assert pat.search(s), name
    s = pat.sub(new, s, count=1)
repl('Server1', '<measuringPoints xsi:type="pcmmeasuringpoint:ExternalCallActionMeasuringPoint" stringRepresentation="Server1">\n    <externalCall href="loadbalancer.repository#_8nlBcAEoEeS7FKokKTKFow"/>\n  </measuringPoints>')
repl('Server2', '<measuringPoints xsi:type="pcmmeasuringpoint:ExternalCallActionMeasuringPoint" stringRepresentation="Server2">\n    <externalCall href="loadbalancer.repository#_-fSxAAEoEeS7FKokKTKFow"/>\n  </measuringPoints>')
repl('overloadUsageScenario', '<measuringPoints xsi:type="pcmmeasuringpoint:UsageScenarioMeasuringPoint" stringRepresentation="overloadUsageScenario">\n    <usageScenario href="overload.usagemodel#_YtqF4AEqEeS7FKokKTKFow"/>\n  </measuringPoints>')
for pdu in ('rackPdu', 'loadBalancerPdu', 'Server2Pdu'):
    repl(pdu, '<measuringPoints xsi:type="org.palladiosimulator.edp2.models:ResourceURIMeasuringPoint" stringRepresentation="%s" measuringPoint="%s" resourceURI="power-extension-removed"/>' % (pdu, pdu))
s = s.replace(P, '')
open(mp, 'w').write(s)
mr = d + '/monitor.monitorrepository'
m = open(mr).read()
for mon in ('rackPduMonitor', 'loadBalancerPduMonitor', 'server2PduMonitor'):
    pat = re.compile(r'\s*<monitors [^>]*entityName="%s">.*?</monitors>' % mon, re.S)
    assert pat.search(m), mon
    m = pat.sub('', m, count=1)
for i, ty in ((6, 'pcmmeasuringpoint:ExternalCallActionMeasuringPoint'), (7, 'pcmmeasuringpoint:ExternalCallActionMeasuringPoint'),
              (8, 'pcmmeasuringpoint:UsageScenarioMeasuringPoint')):
    m = m.replace('<measuringPoint xsi:type="org.palladiosimulator.edp2.models:ResourceURIMeasuringPoint" href="loadBalancerMeasuringpointRepo.measuringpoint#//@measuringPoints.%d"/>' % i,
                  '<measuringPoint xsi:type="%s" href="loadBalancerMeasuringpointRepo.measuringpoint#//@measuringPoints.%d"/>' % (ty, i))
open(mr, 'w').write(m)
EOF
# 5. SimuLizar's ReconfigurationSimulationTest (SIMULIZAR-169): adapt.qvto increments a resource
#    demand at every reconfiguration; the n-th user's response time is n (10 users).
import sl_reconfiguration "$TM/reconfiguration/default.usagemodel,$TM/reconfiguration/default.allocation,$TM/reconfiguration/default.monitorrepository" \
	"$(runjson 1 10 -1)" "$TM/reconfiguration/reconfigs/adapt.qvto"
