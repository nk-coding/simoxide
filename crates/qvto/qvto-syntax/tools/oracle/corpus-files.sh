#!/bin/sh
# Lists the parser corpus: every .qvto of the Eclipse QVTo checkout (tests and plugins), of
# SimuLizar and of the SPD semantic transformations. Paths are absolute.
#   QVTO_ECLIPSE_DIR   default: <repo>/reference/eclipse-qvto
#   QVTO_SIMULIZAR_DIR default: <palladio-research>/repos/Palladio-Analyzer-SimuLizar
#   QVTO_SPD_DIR       default: <palladio-research>/repos/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)  # the QVTo root, crates/qvto
# <palladio-research>: reference/tools/palladio-research.sh ($PALLADIO_RESEARCH, else reference/.work/palladio-research)
PR=$("$ROOT/../../reference/tools/palladio-research.sh")
E=${QVTO_ECLIPSE_DIR:-$ROOT/reference/eclipse-qvto}
S=${QVTO_SIMULIZAR_DIR:-$PR/repos/Palladio-Analyzer-SimuLizar}
P=${QVTO_SPD_DIR:-$PR/repos/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations}
{
  find -L "$E/tests/" "$E/plugins/" -type f -name '*.qvto'
  [ -d "$S" ] && find "$S" -type f -name '*.qvto'
  [ -d "$P" ] && find "$P" -type f -name '*.qvto'
} | sort
