# Paths and command lines shared by the bench/compare scripts (sourced).
C="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
R="$(cd "$C/../.." && pwd)"
PR=${PALLADIO_RESEARCH:-/home/devbox/workspace/palladio-research}
WS=$PR/work-simucom
WSL=$PR/work-slingshot
PLUGINS=${PALLADIO_PLUGINS:-$WS/palladio-5.2.2/plugins}
JAVA=${JAVA:-/usr/bin/java}
B="${CMP_BUILD:-$C/build}"
# stock SimuLizar flat classpath = refsim's OSGi-ordered product classpath (commons-math 2.1.0 before desmoj's copy)
SL_CP="$(cat "$R/reference/build/classpath.txt" 2>/dev/null)"
VT_JAR=$WS/patch-vt/de.uka.ipd.sdq.simulation.abstractsimengine_5.2.2.jar
SS_CP="$(ls $WSL/sl-install/plugins/*.jar $WSL/nested/*.jar | grep -v -E 'equinox.launcher|swt' | tr '\n' ':')"
SIMOXIDE="$R/target/release/simoxide"
SIMOXIDE_DRV="${CMP_CARGO_TARGET:-$R/target/compare}/release/simoxide-compare"
REFSIM_CP="$B/refsim:$R/reference/build/classes-patches:$R/reference/build/classes-base:$SL_CP"
