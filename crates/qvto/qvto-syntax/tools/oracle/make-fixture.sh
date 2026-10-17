#!/bin/sh
# Regenerates tests/fixtures/eclipse-parse-oracle.tsv: Eclipse's parse-only problems for the
# whole corpus, with paths relative to the corpus roots (eclipse:, simulizar:, spd:).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)  # the QVTo root, crates/qvto
E=$(cd "${QVTO_ECLIPSE_DIR:-$ROOT/reference/eclipse-qvto}" && pwd -P)
S=$(cd "${QVTO_SIMULIZAR_DIR:-$ROOT/../../../old/palladio-research/repos/Palladio-Analyzer-SimuLizar}" 2>/dev/null && pwd -P || echo /nonexistent)
P=$(cd "${QVTO_SPD_DIR:-$ROOT/../../../old/palladio-research/repos/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations}" 2>/dev/null && pwd -P || echo /nonexistent)
QVTO_ECLIPSE_DIR=$E QVTO_SIMULIZAR_DIR=$S QVTO_SPD_DIR=$P "$HERE/corpus-files.sh" \
  | "$HERE/run.sh" \
  | sed -e "s|^$E/|eclipse:|" -e "s|^$S/|simulizar:|" -e "s|^$P/|spd:|" \
  > "$HERE/../../tests/fixtures/eclipse-parse-oracle.tsv"
wc -l "$HERE/../../tests/fixtures/eclipse-parse-oracle.tsv"
