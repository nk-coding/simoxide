#!/usr/bin/env python3
"""Adds regression cases to cases/fuzz.cases: every X.qvto of the given directories (with X.xmi
as its input when present), copied into fuzz/<name>/ (name: the directory's name, or
--name). Run from the QVTo root (crates/qvto), then `qvto conform record --suite fuzz --only ...`.
  add-fuzz-case.py [--name N] [--note TEXT] DIR...
"""
import os, shutil, sys

args = sys.argv[1:]
name = note = None
dirs = []
while args:
    a = args.pop(0)
    if a == '--name':
        name = args.pop(0)
    elif a == '--note':
        note = args.pop(0)
    else:
        dirs.append(a)
cases = 'qvto-testkit/cases/fuzz.cases'
text = open(cases, encoding='utf-8').read()
added = []
for d in dirs:
    n = name or os.path.basename(os.path.normpath(d))
    dst = os.path.join('qvto-testkit/fuzz', n)
    os.makedirs(dst, exist_ok=True)
    for f in sorted(os.listdir(d)):
        if not f.endswith('.qvto'):
            continue
        stem = f[:-5]
        cid = f'fuzz_{n}' if stem == n else f'fuzz_{n}_{stem}'
        if f'case {cid}\n' in text:
            continue
        shutil.copy(os.path.join(d, f), dst)
        block = [f'case {cid}', 'driver fuzz', 'kind log', 'features fuzz', f'dir {dst}', f'qvto {f}']
        if os.path.exists(os.path.join(d, stem + '.xmi')):
            shutil.copy(os.path.join(d, stem + '.xmi'), dst)
            block.append(f'in {stem}.xmi')
        block.append('mmfile ../fuzz.ecore')
        if note:
            block.append(f'note {note}')
        block.append('end')
        text += '\n' + '\n'.join(block) + '\n'
        added.append(cid)
open(cases, 'w', encoding='utf-8').write(text)
print(','.join(added))
