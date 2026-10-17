#!/usr/bin/env python3
"""Path coverage of the SPD adjustment corpus (DESIGN-SPD Q1): runs `qvto conform --suite spd` with
QVTO_COVERAGE and prints, per operation of the SPD transformations, the number of cases that
reach it (`entries - rejected > 0`: a mapping whose guard passed), then the cases per operation
and the operations no case reaches. Run from the QVTo root (crates/qvto):

    qvto-testkit/tools/spd-coverage.py [--per-case] [qvto binary]
"""
import collections
import glob
import os
import subprocess
import sys
import tempfile

per_case = "--per-case" in sys.argv
args = [a for a in sys.argv[1:] if a != "--per-case"]
qvto = args[0] if args else "../../target/release-unwind/qvto"
d = tempfile.mkdtemp(prefix="spd-cov-")
subprocess.run([qvto, "conform", "--suite", "spd"], env=dict(os.environ, QVTO_COVERAGE=d),
               check=True, stdout=subprocess.DEVNULL)
ops = {}
reached = collections.defaultdict(list)
rejected_only = collections.defaultdict(list)
cases = sorted(glob.glob(os.path.join(d, "*.tsv")))
per = {}
for f in cases:
    case = os.path.basename(f)[:-4]
    per[case] = []
    for line in open(f).read().splitlines()[1:]:
        kind, op, entries, rejected = line.split("\t")
        if "ProfilesLibrary" in op:
            continue
        ops[op] = kind
        e, r = int(entries), int(rejected)
        if e - r > 0:
            reached[op].append(case)
            per[case].append(op)
        elif e > 0:
            rejected_only[op].append(case)
unit = lambda op: op.rsplit("(", 1)[1].split(":")[0]
print(f"{len(cases)} cases; {len(ops)} operations, {len(reached)} reached\n")
print("| unit | operation | kind | cases |")
print("|---|---|---|---:|")
for op in sorted(ops, key=lambda o: (unit(o), int(o.rsplit(":", 1)[1][:-1]))):
    name = op.split(" (")[0]
    n = len(reached.get(op, []))
    extra = f" (guard only: {len(rejected_only[op])})" if op in rejected_only and not n else ""
    print(f"| {unit(op)} | `{name}` | {ops[op]} | {n}{extra} |")
if per_case:
    print()
    for case, l in per.items():
        print(f"{case}: {len(l)} operations")
        for op in l:
            print(f"  {op.split(' (')[0]}")
