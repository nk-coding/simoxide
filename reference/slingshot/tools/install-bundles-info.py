#!/usr/bin/env python3
"""Generate the Equinox configuration (config.ini + simpleconfigurator bundles.info) of a fresh p2 install
(install.sh step 4; from old/palladio-research/work-slingshot/make-config.py). Writes into INSTALL/configuration."""
import os, re, sys, zipfile, glob
root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else 'sl-install')
start = {'org.eclipse.equinox.common': 2, 'org.apache.felix.scr': 2, 'org.eclipse.equinox.event': 2,
         'org.eclipse.core.runtime': 4, 'org.eclipse.equinox.simpleconfigurator': 1}
def manifest(p):
    if os.path.isdir(p):
        mf = os.path.join(p, 'META-INF', 'MANIFEST.MF')
        if not os.path.exists(mf): return None
        txt = open(mf, encoding='utf-8', errors='replace').read()
    else:
        try:
            with zipfile.ZipFile(p) as z: txt = z.read('META-INF/MANIFEST.MF').decode('utf-8', 'replace')
        except Exception: return None
    txt = re.sub(r'\r?\n ', '', txt)
    h = dict(l.split(':', 1) for l in txt.splitlines() if ':' in l)
    bsn = h.get('Bundle-SymbolicName'); 
    if not bsn: return None
    return bsn.split(';')[0].strip(), h.get('Bundle-Version', '0.0.0').strip()
lines, fw, sc = [], None, None
excl = re.compile(os.environ['EXCLUDE']) if os.environ.get('EXCLUDE') else None
for p in sorted(glob.glob(os.path.join(root, 'plugins', '*'))):
    m = manifest(p)
    if not m: continue
    bsn, ver = m
    if excl and excl.search(bsn): continue
    rel = 'plugins/' + os.path.basename(p)
    if bsn == 'org.eclipse.osgi': fw = rel; continue
    if bsn == 'org.eclipse.equinox.simpleconfigurator': sc = rel
    lvl = start.get(bsn, 4)
    lines.append(f'{bsn},{ver},{rel},{lvl},{"true" if bsn in start else "false"}')
cfg = os.path.join(root, 'configuration')
os.makedirs(os.path.join(cfg, 'org.eclipse.equinox.simpleconfigurator'), exist_ok=True)
open(os.path.join(cfg, 'org.eclipse.equinox.simpleconfigurator', 'bundles.info'), 'w').write(
    '#version=1\n' + '\n'.join(lines) + '\n')
open(os.path.join(cfg, 'config.ini'), 'w').write(f"""osgi.framework=file\\:{fw}
osgi.bundles=reference\\:file\\:{os.path.basename(sc)}@1\\:start
org.eclipse.equinox.simpleconfigurator.configUrl=file\\:org.eclipse.equinox.simpleconfigurator/bundles.info
osgi.bundles.defaultStartLevel=4
eclipse.ignoreApp=false
osgi.noShutdown=false
""")
print(len(lines), 'bundles; framework', fw)
