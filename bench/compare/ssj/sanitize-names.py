#!/usr/bin/env python3
"""Copies a PCM model directory and makes every entityName a valid part of a Java identifier, for SimuCom.

SimuCom generates Java classes, fields and methods from entity names. Its JavaNamesExt.javaName() replaces only
" <>:.,+-() and spaces, so names such as `createUser@user` (operation signatures of the MiSArch model),
`broker→address`, a signature named `sidecar media` (signature names are used verbatim) or a component named
`return` give code that does not compile. Mapping: '@' -> '_at_', '→' -> '_to_', other characters outside
[A-Za-z0-9_] (also spaces) -> '_', Java keywords get a trailing '_'. Only entityName
attributes change; ids, references and specifications stay, so the simulation is the same (ssj/smoke.sh misarch
compares SimOxide on both copies).

usage: sanitize-names.py SRC_DIR DST_DIR"""
import os, re, shutil, sys

src, dst = sys.argv[1], sys.argv[2]
if os.path.exists(dst):
    shutil.rmtree(dst)
shutil.copytree(src, dst)


JAVA_KEYWORDS = set('''abstract assert boolean break byte case catch char class const continue default do double else
enum extends final finally float for goto if implements import instanceof int interface long native new package private
protected public return short static strictfp super switch synchronized this throw throws transient try void volatile
while true false null var record yield'''.split())


def clean(name):
    name = name.replace('@', '_at_').replace('→', '_to_')
    name = re.sub(r'[^A-Za-z0-9_]', '_', name)
    return name + '_' if name in JAVA_KEYWORDS else name   # e.g. the MiSArch service `return`


changed = 0
for f in sorted(os.listdir(dst)):
    p = os.path.join(dst, f)
    if not os.path.isfile(p) or f.endswith('.json'):
        continue
    text = open(p, encoding='utf-8').read()

    def sub(m):
        global changed
        new = clean(m.group(1))
        changed += new != m.group(1)
        return 'entityName="' + new + '"'

    out = re.sub(r'entityName="([^"]*)"', sub, text)
    if out != text:
        open(p, 'w', encoding='utf-8').write(out)
print(f'{changed} entity names changed -> {dst}')
