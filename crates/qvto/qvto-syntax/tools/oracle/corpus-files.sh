#!/bin/sh
# Lists the parser corpus: every .qvto of the Eclipse QVTo checkout (tests and plugins), of
# SimuLizar and of the SPD semantic transformations. Paths are absolute.
#   QVTO_ECLIPSE_DIR   default: <repo>/reference/eclipse-qvto
#   QVTO_SIMULIZAR_DIR default: <workspace>/old/palladio-research/repos/Palladio-Analyzer-SimuLizar
#   QVTO_SPD_DIR       default: <workspace>/old/palladio-research/repos/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)
E=${QVTO_ECLIPSE_DIR:-$ROOT/reference/eclipse-qvto}
S=${QVTO_SIMULIZAR_DIR:-$ROOT/../old/palladio-research/repos/Palladio-Analyzer-SimuLizar}
P=${QVTO_SPD_DIR:-$ROOT/../old/palladio-research/repos/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations}
{
  find -L "$E/tests/" "$E/plugins/" -type f -name '*.qvto'
  [ -d "$S" ] && find "$S" -type f -name '*.qvto'
  [ -d "$P" ] && find "$P" -type f -name '*.qvto'
} | sort
