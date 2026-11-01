# Sourced by the ssj/*.sh scripts: finds the Palladio installs, selects the SSJ extension and loads lib.sh.
# The Palladio installs live in the palladio-research work directory (reference/tools/palladio-research.sh:
# $PALLADIO_RESEARCH if set, else reference/.work/palladio-research).
SSJ_HOME="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PALLADIO_RESEARCH="$("$SSJ_HOME/../../../reference/tools/palladio-research.sh")"
export PALLADIO_RESEARCH
export CMP_EXT="$SSJ_HOME/ext.sh"
export SCRATCH=${SCRATCH:-$SSJ_HOME/results/tmp}
source "$SSJ_HOME/../lib.sh"
