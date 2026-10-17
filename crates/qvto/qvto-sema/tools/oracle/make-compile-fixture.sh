#!/bin/sh
# Regenerates Eclipse QVTo's full-compile oracle for the parse corpus:
#   tests/fixtures/eclipse-compile-oracle.tsv   one line per compile problem (the unit and its imports)
#       <key> E|W <unit file> <line> <offset> <length> <message>
#     (offset/length: UTF-16 char offsets, as Eclipse gives them; message escaped as the runner's
#     diag.txt: \\ \n \t \r), "<key> OK" for a file without problems, and per file
#       <key> LOAD <severity of loadTransformation>     (OK, WARNING, ERROR; a library is not a transformation)
#       <key> RUNNER <verdict> <detail>                 only if the runner failed (error, timeout)
#   tests/fixtures/eclipse-compile-times.tsv    <key> <compile ms> (loadTransformation, warm JVM, one run)
# from tests/data/compile.cases (tools/oracle/gen_compile_cases.py). Needs the runner
# (reference/runner/build.sh, built here if missing), the PCM metamodels at target/pcm
# (crates/qvto-emf/tools/pcm-setup.sh target/pcm, or a symlink to an existing one) and the
# SimuLizar/SPD repositories under ../old/palladio-research/repos.
#   KEEP=dir   keep the runner's result directories there (default: a temporary directory)
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../../.." && pwd)
CASES=$ROOT/crates/qvto-sema/tests/data/compile.cases
FIX=$ROOT/crates/qvto-sema/tests/fixtures
RUNNER=$ROOT/reference/runner
[ -f "$RUNNER/build/classpath.txt" ] || "$RUNNER/build.sh"
[ -f "$ROOT/target/pcm/pcm.conf" ] || { echo "missing target/pcm/pcm.conf (crates/qvto-emf/tools/pcm-setup.sh target/pcm)" >&2; exit 1; }
python3 -I "$HERE/gen_compile_cases.py"

# Extra classes on the runner's classpath: the SimuLizar/SPD blackbox stubs (registered by the
# cases) and the Java libraries that the JDT tests Bug578265/Bug579914 compile in their workspace
# (resolved by class name, as the workspace's Java project provides them).
STUBS=$ROOT/target/qvto-sema-oracle/stubs
T=$ROOT/reference/eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/sources
rm -rf "$STUBS" && mkdir -p "$STUBS"
CP=$(sed "s#\(^\|:\)lib/#\1$RUNNER/lib/#g" "$RUNNER/build/classpath.txt")
nice -n 19 javac -nowarn -encoding UTF-8 -proc:none -d "$STUBS" -cp "$RUNNER/build/classes:$CP" \
	$(find "$HERE/stubs" "$T/bug578265/src" "$T/bug579914/src" -name '*.java')

OUT=${KEEP:-$(mktemp -d)}
mkdir -p "$OUT"
# Every case twice in one JVM: a warm-up pass (ids warm__*, ignored) and the recorded pass, so the
# compile times are those of a warm JVM.
sed 's/^case /case warm__/' "$CASES" > "$OUT/batch.cases"
cat "$CASES" >> "$OUT/batch.cases"
EXTRA_CP=$STUBS XMX=-Xmx2g "$RUNNER/qvto-run" --batch "$OUT/batch.cases" --out "$OUT" --base "$ROOT" --no-tape --timeout 300 > "$OUT/batch.log"
tail -1 "$OUT/batch.log"

mkdir -p "$FIX"
python3 -I - "$CASES" "$OUT" "$FIX" <<'EOF'
import os, sys
cases, out, fix = sys.argv[1:4]
keys = []   # (case id, key)
cid = None
for line in open(cases, encoding='utf-8'):
    line = line.strip()
    if line.startswith('case '):
        cid = line[5:]
    elif line.startswith('note key '):
        keys.append((cid, line[9:]))
rows = {}
with open(os.path.join(out, 'results.tsv'), encoding='utf-8') as f:
    head = f.readline().rstrip('\n').split('\t')
    for l in f:
        r = dict(zip(head, l.rstrip('\n').split('\t')))
        rows[r['id']] = r
oracle, times = [], []
for cid, key in sorted(keys, key=lambda x: x[1]):
    r = rows.get(cid)
    if r is None:
        sys.exit(f'no result for {cid}')
    probs = []
    p = os.path.join(out, cid, 'diag.txt')
    if os.path.exists(p):
        for l in open(p, encoding='utf-8').read().split('\n'):
            f = l.split('\t')
            if f[0] == 'problem':
                probs.append('\t'.join([key, 'E' if f[1] == 'error' else 'W'] + f[2:]))
    oracle.extend(probs or [f'{key}\tOK'])
    oracle.append(f'{key}\tLOAD\t{r["load"]}')
    if r['verdict'] not in ('pass', 'fail'):
        oracle.append(f'{key}\tRUNNER\t{r["verdict"]}\t{r["detail"]}')
    times.append(f'{key}\t{r["compile_ms"]}')
with open(os.path.join(fix, 'eclipse-compile-oracle.tsv'), 'w', encoding='utf-8') as f:
    f.write('\n'.join(oracle) + '\n')
with open(os.path.join(fix, 'eclipse-compile-times.tsv'), 'w', encoding='utf-8') as f:
    f.write('\n'.join(times) + '\n')
print(f'{len(keys)} files, {sum(1 for l in oracle if l.split(chr(9))[1] in ("E", "W"))} problems')
EOF
[ -n "$KEEP" ] || rm -rf "$OUT"
