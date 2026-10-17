#!/bin/sh
# Copies the SimuLizar and SPD test material for cases/corpus.cases into corpus/ and writes the
# synthesized runtime-measurement inputs. Run from the QVTo root (crates/qvto):
#   qvto-testkit/tools/copy-corpus.sh [repos-dir]
# Sources (EPL-2.0), revisions used for the committed copy:
#   Palladio-Analyzer-SimuLizar        2718d73a1f236d5cae10313c2022c6528a99e805
#   Palladio-Addons-SPD-Metamodel      95cdb0b677276f5d14c3001b89705a17d8fb690f
#   Palladio-Documentation-Slingshot   94448159e75f33edfb98e960ad244018b97e37d4
set -e
R=${1:-/home/devbox/workspace/old/palladio-research/repos}
S=$R/Palladio-Analyzer-SimuLizar
T=$S/tests/org.palladiosimulator.simulizar.tests/testmodels
L=$S/bundles/ExampleModels/org.palladiosimulator.simulizar.examples.loadbalancer
O=qvto-testkit/corpus
rm -rf $O/simulizar $O/spd
mkdir -p $O/simulizar/runconfigtest/monitors $O/simulizar/reconfiguration $O/simulizar/loadbalancer/monitors \
	$O/spd/minimalexample

# SimuLizar's QVToReconfigurationTest models and rules
cp $T/runconfigtest/server.* $O/simulizar/runconfigtest/
cp $T/runconfigtest/monitors/server.* $O/simulizar/runconfigtest/monitors/
for r in outsource scaleUp addNewServer addClonedServer; do
	cp $T/runconfigtest/rules/$r/$r.qvto $O/simulizar/runconfigtest/
done
cp $T/reconfiguration/default.* $T/reconfiguration/My.resourceenvironment $T/reconfiguration/reconfigs/adapt.qvto \
	$O/simulizar/reconfiguration/
# the loadbalancer example and its QVTo rules
cp $L/loadbalancer.repository $L/loadbalancer.system $L/iaas.resourceenvironment $L/loadbalancer_on_iaas.allocation \
	$L/overload.usagemodel $O/simulizar/loadbalancer/
cp $L/monitors/sensors.measuringpoint $L/monitors/monitoring.monitorrepository $O/simulizar/loadbalancer/monitors/
for r in outsource scaleDown scaleIn scaleUp; do cp $L/reconfigurations_qvto/$r.qvto $O/simulizar/loadbalancer/; done
# the power-consumption variant's PCM-only rules (its Power models are not needed by them)
W=$S/bundles/ExampleModels/TransientEffects/org.palladiosimulator.simulizar.examples.loadbalancer.powerconsumption
mkdir -p $O/simulizar/powerconsumption/monitors
cp $W/loadbalancer.repository $W/loadbalancer.system $W/iaas.resourceenvironment $W/loadbalancer_on_iaas.allocation \
	$W/overload.usagemodel $O/simulizar/powerconsumption/
cp $W/monitors/* $O/simulizar/powerconsumption/monitors/
for r in scaleDown scaleIn scaleUp; do cp $W/reconf_qvto/$r.qvto $O/simulizar/powerconsumption/; done

# Runtime measurements (SimuLizar's PRM): no serialized ones exist; SimuLizar binds the
# measurements of its RuntimeMeasurementModel as the extent, so the files hold
# RuntimeMeasurement roots. Values as in QVToReconfigurationTest (5.0 above, 1.0 below the
# rules' threshold 3.0) and below the example rules' scale-down threshold (0.5). The measuring
# point is the first of the model's measuring-point repository (its xsi:type copied).
mptype() { grep -o '<measuringPoints[^>]*xsi:type="[^"]*"' "$1" | head -1 | sed 's/.*xsi:type="\([^"]*\)".*/\1/'; }
prm() { # file value measuringpoint-href spec-href measuringpoint-file
	TY=$(mptype "$5")
	PFX=${TY%%:*}
	NS=$(grep -o "xmlns:$PFX=\"[^\"]*\"" "$5" | head -1)
	cat > "$1" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<xmi:XMI xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" $NS xmlns:srm="http://simulizar.palladiosimulator.org/RuntimeMeasurement/1.0">
  <srm:RuntimeMeasurement id="_prm_measurement_1" measuringValue="$2">
    <measuringPoint xsi:type="$TY" href="$3"/>
    <measurementSpecification href="$4"/>
  </srm:RuntimeMeasurement>
</xmi:XMI>
EOF
}
MP=monitors/server.measuringpoint#//@measuringPoints.0
SP=monitors/server.monitorrepository#_aG74UC1SEeaoQ9JnkNiADA
prm $O/simulizar/runconfigtest/prm-high.xmi 5.0 $MP $SP $T/runconfigtest/monitors/server.measuringpoint
prm $O/simulizar/runconfigtest/prm-low.xmi 1.0 $MP $SP $T/runconfigtest/monitors/server.measuringpoint
prm $O/simulizar/reconfiguration/prm.xmi 5.0 default.measuringpoint#//@measuringPoints.0 \
	"default.monitorrepository#$(grep -o '<measurementSpecifications id="[^"]*"' $T/reconfiguration/default.monitorrepository | head -1 | cut -d'"' -f2)" \
	$T/reconfiguration/default.measuringpoint
LSP="monitors/monitoring.monitorrepository#$(grep -o '<measurementSpecifications[^>]*id="[^"]*"' $L/monitors/monitoring.monitorrepository | head -1 | sed 's/.*id="\([^"]*\)".*/\1/')"
LMT=$L/monitors/sensors.measuringpoint
prm $O/simulizar/loadbalancer/prm-high.xmi 5.0 monitors/sensors.measuringpoint#//@measuringPoints.0 "$LSP" $LMT
prm $O/simulizar/loadbalancer/prm-low.xmi 0.5 monitors/sensors.measuringpoint#//@measuringPoints.0 "$LSP" $LMT

PMP=monitors/loadBalancerMeasuringpointRepo.measuringpoint
PSP="monitors/monitor.monitorrepository#$(grep -o '<measurementSpecifications[^>]*id="[^"]*"' $W/monitors/monitor.monitorrepository | head -1 | sed 's/.*id="\([^"]*\)".*/\1/')"
prm $O/simulizar/powerconsumption/prm-high.xmi 5.0 "$PMP#//@measuringPoints.0" "$PSP" $W/$PMP
prm $O/simulizar/powerconsumption/prm-low.xmi 0.5 "$PMP#//@measuringPoints.0" "$PSP" $W/$PMP

# SPD semantic transformations and Slingshot's minimal example
cp -r $R/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations/transformations $O/spd/
rm -rf $O/spd/transformations/archive
M=$R/Palladio-Documentation-Slingshot/examples/minimalexample
cp $M/default.allocation $M/default.system $M/default.repository $M/default.usagemodel $M/My.resourceenvironment \
	$M/default.spd $M/default.measuringpoint $M/default.monitorrepository $O/spd/minimalexample/
# Slingshot sets the enacted policy before running MainTransformation (SpdAdjustmentBehavior)
sed 's#<spd href="default.spd\#_PKdecLwZEe6i2ICnm9lfkw"/>#&\n  <enactedPolicy href="default.spd\#_7VIU0MAgEe6AcKIPVXJsSw"/>#' \
	$M/default.semanticspd > $O/spd/minimalexample/default.semanticspd
grep -q enactedPolicy $O/spd/minimalexample/default.semanticspd
du -sh $O
