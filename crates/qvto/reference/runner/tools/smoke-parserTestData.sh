#!/bin/bash
# Smoke: runs every parserTestData/models dir with one .qvto, one in.*, one expected.* through the Java runner.
R=$(cd "$(dirname "$0")/.." && pwd)
M=$R/../eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/models
O=${1:?usage: smoke-parserTestData.sh OUTDIR}; mkdir -p $O
for d in $M/*/; do
  n=$(basename $d)
  q=$(ls $d*.qvto 2>/dev/null); i=$(ls $d | grep '^in\.' ); e=$(ls $d | grep '^expected\.')
  [ $(echo "$q" | grep -c .) = 1 ] && [ $(echo "$i" | grep -c .) = 1 ] && [ $(echo "$e" | grep -c .) = 1 ] || continue
  mms=$(ls $d*.ecore 2>/dev/null | grep -vE '/(in|expected)\.ecore$' | tr '\n' ',')
  timeout 60 $R/qvto-run --qvto $q ${mms:+--mm $mms} --param in:$d$i --param out:$O/$n.out.xmi --expect $d$e > $O/$n.txt 2>&1
  echo "$n $?"
done > $O/summary.txt
