#!/bin/bash
# Builds the Slingshot reference runner against an existing headless Slingshot install (read only) and
# writes private Equinox configurations. Everything generated goes to build/ (gitignored).
#
#   ./setup.sh            build bundles + seed agent + patched jars + configurations, verify install.lock
#   ./setup.sh --lock     (re)write install.lock from the install found (after a deliberate update)
#
# Env: SLINGSHOT_INSTALL (default: first existing of the locations below), JAVAC, JAR.
# The install itself is made by install.sh (p2 director, network); see docs/reference-simulator/slingshot.md.
set -euo pipefail
H="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$H/env.sh"
JAVAC=${JAVAC:-javac}
JAR=${JAR:-jar}

[ -n "$I" ] || { echo "no Slingshot install found; set SLINGSHOT_INSTALL (see install.sh)" >&2; exit 1; }
echo "install: $I"

# --- 0. pin: checksums of the Slingshot, SPD, QVTo and simulation-framework jars ----------------------
lockfile() {
  (cd "$I/plugins" && ls | grep -E '^(org\.palladiosimulator\.(analyzer\.slingshot|spd)|org\.eclipse\.m2m\.qvt\.oml|de\.uka\.ipd\.sdq\.(simucomframework|probfunction)|ca\.umontreal\.iro\.simul\.ssj)' \
     | sort | xargs sha256sum)
}
if [ "${1:-}" = --lock ]; then
  { echo "# sha256 of the pinned jars of $I (setup.sh --lock, $(date -I))"; lockfile; } > "$H/install.lock"
  echo "wrote install.lock"; exit 0
fi
mkdir -p "$B"
lockfile > "$B/lock.now"
if ! diff <(grep -v '^#' "$H/install.lock") "$B/lock.now" > "$B/lock.diff"; then
  echo "WARNING: install differs from install.lock (see $B/lock.diff); results may not be comparable" >&2
fi

rm -rf "$B/classes" "$B/plugins" "$B/patched" "$B/equinox" "$B/osgi"
mkdir -p "$B"/{classes,plugins,patched,nested,equinox}

# --- 1. compile classpath: bundle jars + their nested library jars ------------------------------------
if [ ! -f "$B/nested/.done" ]; then
  for j in "$I"/plugins/*.jar; do
    unzip -Z1 "$j" 2>/dev/null | grep -E '^(lib/)?[^/]*\.jar$' | while read -r n; do
      unzip -o -q -j "$j" "$n" -d "$B/nested/" ; done || true
  done
  touch "$B/nested/.done"
fi
CP="$(ls "$I"/plugins/*.jar "$B"/nested/*.jar | grep -v -E 'slingshot\.headless|equinox\.launcher|/org\.eclipse\.swt' | tr '\n' ':')"

bundle() { # src-dir jar-name [extra classpath]
  local s="$H/src/$1" c="$B/classes/$1"
  mkdir -p "$c"
  $JAVAC -nowarn -proc:none --release 21 -d "$c" -cp "$CP${3:-}" $(find "$s/src" -name '*.java')
  cp "$s/plugin.xml" "$c/"
  $JAR cfm "$B/plugins/$2" "$s/META-INF/MANIFEST.MF" -C "$c" .
  echo "built $2"
}
bundle stub org.palladiosimulator.architecturaltemplates.catalog_5.2.2.stub.jar
bundle instrument simoxide.slingshot.instrument_1.0.0.jar
bundle runner simoxide.slingshot.runner_1.0.0.jar ":$B/plugins/simoxide.slingshot.instrument_1.0.0.jar"

# --- 2. seed agent (-javaagent; on the boot class path together with ASM, boot-delegated by Equinox) ---
"$H/seed-agent.sh" >/dev/null
echo "built seed-agent.jar"

# --- 3. patched bundles (deterministic same-time order; patches/README.md) -----------------------------
REPLACE=()
for pd in "$H"/patches/*/; do
  bsn=$(basename "$pd"); [ -d "$pd/src" ] || continue
  orig=$(ls "$I"/plugins/${bsn}_*.jar | head -1)
  c="$B/classes/patch-$bsn"; mkdir -p "$c"
  $JAVAC -nowarn -proc:none --release 21 -d "$c" -cp "$CP" $(find "$pd/src" -name '*.java')
  # the unpatched sources (orig/) must match the installed classes (member signatures, javap -p)
  co="$B/classes/orig-$bsn"; mkdir -p "$co"
  $JAVAC -nowarn -proc:none --release 21 -d "$co" -cp "$CP" $(find "$pd/orig" -name '*.java')
  for cls in $(cd "$co" && find . -name '*.class' | sed 's|^\./||;s|\.class$||;s|/|.|g'); do
    if ! diff <(javap -p -cp "$co" "$cls" | grep -v -E '^Compiled|lambda\$' | sort) <(javap -p -cp "$orig" "$cls" | grep -v -E '^Compiled|lambda\$' | sort) >/dev/null; then
      echo "ERROR: patches/$bsn/orig does not match the installed $cls" >&2; exit 1
    fi
  done
  out="$B/patched/$(basename "$orig")"
  cp "$orig" "$out"; chmod u+w "$out"
  (cd "$c" && $JAR uf "$out" $(find . -name '*.class' | sed 's|^\./||'))
  REPLACE+=(--replace "$bsn=$out")
  echo "patched $(basename "$orig"): $(cd "$c" && find . -name '*.class' | wc -l) classes"
done

# --- 4. Equinox configurations: stock (unpatched Slingshot) and det (patched jars) --------------------
OWN=("$B/plugins/"*.jar)
python3 "$H/tools/mkconfig.py" "$I" "$B/equinox/stock" "${OWN[@]}"
python3 "$H/tools/mkconfig.py" "$I" "$B/equinox/det" "${OWN[@]}" "${REPLACE[@]}"

cat > "$B/log4j.properties" <<'EOF'
log4j.rootLogger=WARN, c
log4j.appender.c=org.apache.log4j.ConsoleAppender
log4j.appender.c.layout=org.apache.log4j.PatternLayout
log4j.appender.c.layout.ConversionPattern=%-5p %c{1} - %m%n
EOF
echo "setup done: ./run.sh -model DIR -out DIR -simTime T -seed N"
