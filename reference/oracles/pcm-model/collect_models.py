#!/usr/bin/env python3
"""Writes models.txt (name TAB directory) for every directory containing PCM model files under the
Palladio repos and simoxide/corpus. Names are the directory path relative to its root with '/'
and ' ' replaced by '_' (corpus models are prefixed with 'corpus_', the loader's edge-case models with 'case_')."""
import hashlib, os, sys
EXT = ('.repository', '.system', '.resourceenvironment', '.allocation', '.usagemodel', '.resourcetype',
       '.monitorrepository', '.measuringpoint')
roots = [('', '/home/devbox/workspace/palladio-research/repos'),
         ('corpus_', os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../../corpus'))),
         ('case_', os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../../crates/simoxide-model/tests/xmi-cases')))]
out = []
seen = set()
def digest(d):
    h = hashlib.sha256()
    for f in sorted(os.listdir(d)):
        if f.endswith(EXT) and os.path.isfile(os.path.join(d, f)):
            h.update(f.encode() + b'\0' + open(os.path.join(d, f), 'rb').read())
    return h.hexdigest()
for prefix, root in roots:
    if not os.path.isdir(root):
        continue
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x not in ('.git', 'target', 'bin', 'expected'))
        if any(f.endswith(EXT) for f in files):
            key = digest(d)
            if key in seen:  # identical copy of another model directory
                continue
            seen.add(key)
            rel = os.path.relpath(d, root)
            out.append((prefix + rel.replace('/', '_').replace(' ', '_'), d))
out.sort()
# the bundled default models (pathmap resources are dumped only here)
out.insert(0, ('_bundled', '-'))
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'models.txt'), 'w') as f:
    for n, d in out:
        f.write(f'{n}\t{d}\n')
print(len(out), 'model directories')
