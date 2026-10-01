#!/bin/bash
# Checks that the releases/5.2.2 git sources of every shadowed class (reference/patches/src) match the
# bytecode in the 5.2.2 product jars: compiles the PRISTINE tag source with javac -g, decompiles it and
# the product class with CFR, and diffs the normalised output (whitespace, String.valueOf/(String) casts
# from javac-vs-ECJ string concatenation removed). Prints "<#differing lines> <class>".
# Env: SRC522 (default palladio-research/src-5.2.2), CFR jar, WORK dir.
set -e
R="$(cd "$(dirname "$0")/.." && pwd)"
SRC522=${SRC522:-/home/devbox/workspace/palladio-research/src-5.2.2}
CFR=${CFR:-/home/devbox/workspace/palladio-research/bin/cfr-0.152.jar}
W=${WORK:-$(mktemp -d)}
CP=$(cat "$R/build/classpath.txt")
rm -rf "$W/pristine" "$W/cls" "$W/jarcls" "$W/dec-a" "$W/dec-b"; mkdir -p "$W/pristine" "$W/cls" "$W/jarcls"
for f in $(cd "$R/patches/src" && find . -name '*.java' | sed 's|^\./||'); do
  src=$(find "$SRC522" -path "*/src*/$f" | grep -v /test | head -1)
  mkdir -p "$W/pristine/$(dirname $f)"; cp "$src" "$W/pristine/$f"
  c=${f%.java}.class
  for e in $(echo "$CP" | tr ':' ' '); do
    if [ -f "$e" ] && unzip -l "$e" "$c" >/dev/null 2>&1; then (cd "$W/jarcls" && unzip -o -q "$e" "${f%.java}*.class"); break; fi
  done
done
javac -g --release 17 -nowarn -cp "$CP" -d "$W/cls" $(find "$W/pristine" -name '*.java') 2>&1 | grep -v '^Note' || true
java -jar "$CFR" --outputdir "$W/dec-a" --silent true --comments false $(find "$W/cls" -name '*.class') >/dev/null 2>&1
java -jar "$CFR" --outputdir "$W/dec-b" --silent true --comments false $(find "$W/jarcls" -name '*.class') >/dev/null 2>&1
norm() { sed -e 's/String\.valueOf(\([^()]*\))/\1/g' -e 's/(String)//g' "$1" | tr -d ' \t' | grep -v '^$'; }
for f in $(cd "$W/dec-a" && find . -name '*.java' | sort); do
  n=$(diff <(norm "$W/dec-a/$f") <(norm "$W/dec-b/$f") | grep -c '^[<>]' || true)
  echo "$n ${f#./}"
done
echo "work dir: $W"
