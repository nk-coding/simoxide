#!/bin/bash
# Makes sure reference/build/classpath.txt is up to date and prints its path. Used by build.sh and
# by all oracles/*, so that every Java tool here runs on the same, OSGi-ordered flat classpath.
#   tools/classpath.py gen   : one jar per bundle + nested Bundle-ClassPath jars (build/lib/<bsn>/)
#   tools/classpath.py check : duplicate classes across jars must resolve like OSGi (docs/reference-simulator/patches.md "Classpath")
# Env: PALLADIO_PLUGINS
set -e
R=$(cd "$(dirname "$0")/.." && pwd)
P=${PALLADIO_PLUGINS:-/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2/plugins}
F="$R/build/classpath.txt"
if [ ! -s "$F" ] || [ "$R/tools/classpath.py" -nt "$F" ] || [ "$0" -nt "$F" ]; then
  mkdir -p "$R/build/lib"
  python3 "$R/tools/classpath.py" gen "$P" "$R/build/lib" > "$F.tmp"
  python3 "$R/tools/classpath.py" check "$F.tmp" > "$R/build/classpath-check.txt" \
    || { cat "$R/build/classpath-check.txt" >&2; echo "classpath check FAILED" >&2; rm -f "$F.tmp"; exit 1; }
  mv "$F.tmp" "$F"
fi
echo "$F"
