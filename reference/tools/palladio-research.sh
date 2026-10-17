#!/bin/bash
# Prints the palladio-research work directory (Palladio 5.2.2 product, sources, Slingshot install):
# $PALLADIO_RESEARCH if set, else /home/devbox/workspace/palladio-research, else its new location
# /home/devbox/workspace/old/palladio-research (moved there in 2026-10).
if [ -n "${PALLADIO_RESEARCH:-}" ]; then echo "$PALLADIO_RESEARCH"; exit 0; fi
for d in /home/devbox/workspace/palladio-research /home/devbox/workspace/old/palladio-research; do
  [ -d "$d" ] && { echo "$d"; exit 0; }
done
echo /home/devbox/workspace/palladio-research
