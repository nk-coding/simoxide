#!/usr/bin/env python3
"""Copies a model directory and gives every AcquireAction/ReleaseAction without a resource demand a
ParametricResourceDemand "1" (CPU type). Slingshot reads the number of tokens from it and aborts without
one; SimuLizar and SimOxide ignore it (outputs are byte-identical, checked by run-all.sh check).
usage: add-acquire-demand.py SRC_DIR DST_DIR"""
import os, re, shutil, sys
src, dst = sys.argv[1], sys.argv[2]
shutil.rmtree(dst, ignore_errors=True)
os.makedirs(dst)
DEMAND = ('<resourceDemand_Action>\n          <specification_ParametericResourceDemand specification="1"/>\n'
          '          <requiredResource_ParametricResourceDemand href="pathmap://PCM_MODELS/Palladio.resourcetype#_oro4gG3fEdy4YaaT-RYrLQ"/>\n'
          '        </resourceDemand_Action>\n      </steps_Behaviour>')
n = 0
for f in sorted(os.listdir(src)):
    p = os.path.join(src, f)
    if not os.path.isfile(p) or f == 'run.json':
        continue
    s = open(p, encoding='utf-8').read()
    if f.endswith('.repository'):
        def rep(m):
            global n
            n += 1
            return m.group(1) + '>\n        ' + DEMAND
        s = re.sub(r'(<steps_Behaviour xsi:type="seff:(?:Acquire|Release)Action"[^>]*?)/>', rep, s)
    open(os.path.join(dst, f), 'w', encoding='utf-8').write(s)
print(f"{dst}: {n} acquire/release actions given a demand")
