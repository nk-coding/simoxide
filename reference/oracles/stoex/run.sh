#!/bin/bash
# Regenerates the golden files:
#   cases.jsonl (from gen_cases.py) -> golden/stoex_golden.jsonl  (StoexOracle, real 5.2.2 classes)
#   golden/javamath.txt                                          (JavaMathDump, Math.pow/log)
set -e
cd "$(dirname "$0")"
CP_FILE=${CP_FILE:-$(../../tools/classpath.sh)}
JAVA=${JAVA:-java}
./build.sh
python3 gen_cases.py > build/cases.jsonl
$JAVA -cp "build/classes:$(cat "$CP_FILE")" oracle.StoexOracle build/cases.jsonl golden/varsets.json golden/stoex_golden.jsonl
$JAVA -cp build/classes oracle.JavaMathDump golden/javamath.txt 3000
