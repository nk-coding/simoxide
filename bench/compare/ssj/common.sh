# Sourced by the ssj/*.sh scripts: finds the Palladio installs, selects the SSJ extension and loads lib.sh.
# The comparison scripts default to /home/devbox/workspace/palladio-research, which has since moved to
# /home/devbox/workspace/old/palladio-research; take whichever exists unless PALLADIO_RESEARCH is set.
SSJ_HOME="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -z "${PALLADIO_RESEARCH:-}" ]; then
  for d in /home/devbox/workspace/palladio-research /home/devbox/workspace/old/palladio-research; do
    [ -d "$d/work-simucom/palladio-5.2.2" ] && { PALLADIO_RESEARCH=$d; break; }
  done
fi
export PALLADIO_RESEARCH
export CMP_EXT="$SSJ_HOME/ext.sh"
export SCRATCH=${SCRATCH:-$SSJ_HOME/results/tmp}
source "$SSJ_HOME/../lib.sh"
