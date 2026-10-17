#!/usr/bin/env python3
"""Private Equinox configuration for the Slingshot reference runner.

usage: mkconfig.py INSTALL OUT_DIR BUNDLE_JAR... [--replace BSN=JAR ...]

Reads INSTALL/configuration/org.eclipse.equinox.simpleconfigurator/bundles.info (the p2-generated list),
drops the old bench bundle `slingshot.headless`, replaces bundles named with --replace by patched copies,
appends the runner's own bundles (absolute file: URLs) and writes OUT_DIR/config.ini + OUT_DIR/bundles.info.
Relative entries stay relative to osgi.install.area (= INSTALL, which is only read).
"""
import os, sys, zipfile, re

def manifest(jar):
    with zipfile.ZipFile(jar) as z:
        txt = z.read('META-INF/MANIFEST.MF').decode('utf-8', 'replace')
    txt = re.sub(r'\r?\n ', '', txt)
    h = dict(l.split(':', 1) for l in txt.splitlines() if ':' in l)
    return h['Bundle-SymbolicName'].split(';')[0].strip(), h.get('Bundle-Version', '0.0.0').strip()

args = sys.argv[1:]
replace = {}
while '--replace' in args:
    i = args.index('--replace')
    bsn, jar = args[i + 1].split('=', 1)
    replace[bsn] = os.path.abspath(jar)
    del args[i:i + 2]
inst, out, extra = os.path.abspath(args[0]), args[1], [os.path.abspath(j) for j in args[2:]]
src = os.path.join(inst, 'configuration', 'org.eclipse.equinox.simpleconfigurator', 'bundles.info')
lines, fw, sc = [], None, None
for l in open(src):
    l = l.strip()
    if not l or l.startswith('#'):
        continue
    bsn, ver, loc, lvl, start = l.split(',')
    if bsn == 'slingshot.headless':
        continue
    if bsn == 'org.eclipse.equinox.simpleconfigurator':
        sc = loc
    if bsn in replace:
        _, ver = manifest(replace[bsn])
        loc = 'file:' + replace.pop(bsn)
    lines.append(','.join([bsn, ver, loc, lvl, start]))
if replace:
    sys.exit('not in the install: ' + ', '.join(replace))
for j in extra:
    bsn, ver = manifest(j)
    lines.append(f'{bsn},{ver},file:{j},4,false')
fwjar = [f for f in os.listdir(os.path.join(inst, 'plugins')) if f.startswith('org.eclipse.osgi_')]
assert len(fwjar) == 1 and sc, 'framework or simpleconfigurator not found'
os.makedirs(out, exist_ok=True)
with open(os.path.join(out, 'bundles.info'), 'w') as f:
    f.write('#version=1\n' + '\n'.join(lines) + '\n')
with open(os.path.join(out, 'config.ini'), 'w') as f:
    f.write(f"""osgi.framework=file\\:{inst}/plugins/{fwjar[0]}
osgi.bundles=reference\\:file\\:{inst}/{sc}@1\\:start
org.eclipse.equinox.simpleconfigurator.configUrl=file\\:{os.path.abspath(out)}/bundles.info
osgi.bundles.defaultStartLevel=4
eclipse.ignoreApp=false
osgi.noShutdown=false
org.osgi.framework.bootdelegation=simoxide.slingshot.seed.*
""")
print(f'{out}: {len(lines)} bundles')
