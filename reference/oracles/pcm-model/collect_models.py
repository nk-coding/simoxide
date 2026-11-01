#!/usr/bin/env python3
"""Writes models.txt (name TAB directory) for every directory containing PCM model files under the
Palladio repos (written $PALLADIO_RESEARCH/repos/...) and simoxide/corpus (written relative to this
directory). Names are the directory path relative to its root with '/' and ' ' replaced by '_'
(corpus models are prefixed with 'corpus_', the loader's edge-case models with 'case_')."""
import hashlib, os, sys
EXT = ('.repository', '.system', '.resourceenvironment', '.allocation', '.usagemodel', '.resourcetype',
       '.monitorrepository', '.measuringpoint')
HERE = os.path.dirname(os.path.abspath(__file__))
# the palladio-research work directory (reference/tools/palladio-research.sh): $PALLADIO_RESEARCH,
# else reference/.work/palladio-research of this repository
PR = os.environ.get('PALLADIO_RESEARCH') or os.path.normpath(os.path.join(HERE, '../../.work/palladio-research'))
# (prefix of the names, root directory, how models.txt writes a directory below the root)
roots = [('', PR + '/repos', lambda rel: '$PALLADIO_RESEARCH/repos/' + rel),
         ('corpus_', os.path.normpath(os.path.join(HERE, '../../../corpus')), None),
         ('case_', os.path.normpath(os.path.join(HERE, '../../../crates/simoxide-model/tests/xmi-cases')), None)]
out = []
seen = set()
def digest(d):
    h = hashlib.sha256()
    for f in sorted(os.listdir(d)):
        if f.endswith(EXT) and os.path.isfile(os.path.join(d, f)):
            h.update(f.encode() + b'\0' + open(os.path.join(d, f), 'rb').read())
    return h.hexdigest()
for prefix, root, record in roots:
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
            # directories relative to this one (as in spd-models.txt), or below $PALLADIO_RESEARCH
            out.append((prefix + rel.replace('/', '_').replace(' ', '_'),
                        record(rel) if record else os.path.relpath(d, HERE)))
out.sort()
# the bundled default models (pathmap resources are dumped only here)
out.insert(0, ('_bundled', '-'))
with open(os.path.join(HERE, 'models.txt'), 'w') as f:
    for n, d in out:
        f.write(f'{n}\t{d}\n')
print(len(out), 'model directories')
