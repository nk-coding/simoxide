#!/bin/bash
# Prints the palladio-research work directory (Palladio 5.2.2 product, upstream sources, Slingshot
# install): $PALLADIO_RESEARCH if set, else reference/.work/palladio-research of this repository
# (git-ignored; a link to the directory works). Warns on stderr if the directory does not exist;
# the callers decide whether that is an error.
if [ -n "${PALLADIO_RESEARCH:-}" ]; then
  d=$PALLADIO_RESEARCH
else
  d="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.work/palladio-research"
fi
[ -d "$d" ] || echo "palladio-research.sh: no palladio-research work directory at $d (set PALLADIO_RESEARCH)" >&2
echo "$d"
