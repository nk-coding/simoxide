#!/usr/bin/env python3
"""Flat classpath for the Palladio 5.2.2 product jars, ordered like the OSGi wiring would bind.

  classpath.py gen <pluginsDir> <libDir>      print the classpath (':'-separated); nested
                                              Bundle-ClassPath jars are extracted to <libDir>/<bsn>/
  classpath.py check <classpath.txt> [-v]     list every class that occurs in more than one jar and
                                              check that its first occurrence is the one OSGi binds

Rules (see docs/reference-simulator/patches.md, section "Classpath"):
  1. One jar per bundle symbolic name, highest version (antlr 3 and 4 are kept both: different packages).
  2. A bundle's entries follow its Bundle-ClassPath order ('.' = the bundle jar itself; the bundle jar is
     always on the classpath, first if '.' is not listed).
  3. Bundles in sorted symbolic-name order.
  4. No bundle-private copy may shadow an exported one: an entry that holds a copy of a class whose
     package its own bundle does NOT export, while another bundle exports it, moves behind all other
     entries. (In OSGi such a copy is invisible outside its bundle; e.g. desmoj-2.3.3-core-bin.jar
     embeds an old org.apache.commons.math, but probfunction.math binds org.apache.commons.math 2.1.)
  5. Classes exported by several bundles (a real OSGi choice per importer) must be listed in REVIEWED
     with the reason why the order does not matter; `check` fails on any other such duplicate.
"""
import collections, os, re, sys, zipfile

# package prefix -> reason (duplicates exported by several bundles; reviewed, see docs/reference-simulator/patches.md "Classpath")
REVIEWED = {
    'javax/el/': 'EL API (com.sun.el.javax.el 3.0.0, javax.el-api 3.0.3): JSP/help UI only, not loaded by refsim',
    'javax/servlet/': 'Servlet API (jakarta.servlet-api 4.0.0, javax.servlet 3.1.0): help/Jetty only, not loaded by refsim',
    'org/eclipse/jdt/': 'JDT compiler copy embedded in org.apache.jasper.glassfish (JSP compiler): not loaded by refsim',
}


def manifest(path):
    try:
        mf = zipfile.ZipFile(path).read('META-INF/MANIFEST.MF').decode('utf-8', 'replace')
    except Exception:
        return None
    mf = re.sub(r'\r?\n ', '', mf)
    return {m.group(1): m.group(2).strip() for m in re.finditer(r'^([A-Za-z0-9_-]+):\s*(.*)$', mf, re.M)}


def split_clauses(s):
    """Split a manifest header on commas outside quotes; return the first element of each clause."""
    out, cur, q = [], '', False
    for ch in s or '':
        if ch == '"':
            q = not q
        if ch == ',' and not q:
            out.append(cur); cur = ''
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return [c.split(';')[0].strip() for c in out if c.strip()]


def ver(v):
    return [int(x) if x.isdigit() else 0 for x in re.split(r'[._-]', v)[:3]]


def classes(path):
    try:
        z = zipfile.ZipFile(path)
    except Exception:
        return []
    return [n for n in z.namelist()
            if n.endswith('.class') and not n.startswith('META-INF/') and os.path.basename(n) != 'module-info.class']


def pkg(cls):
    return os.path.dirname(cls).replace('/', '.')


def gen(plugins, lib):
    best = {}
    for f in sorted(os.listdir(plugins)):
        if not f.endswith('.jar'):
            continue
        mf = manifest(os.path.join(plugins, f))
        if not mf or 'Bundle-SymbolicName' not in mf:
            continue
        bsn = mf['Bundle-SymbolicName'].split(';')[0].strip()
        vv = mf.get('Bundle-Version', '0')
        if bsn == 'org.antlr.runtime':
            bsn += vv  # antlr 3 and 4 use different packages: keep both
        if bsn not in best or ver(vv) > ver(best[bsn][0]):
            best[bsn] = (vv, os.path.join(plugins, f), mf)
    entries = []  # (path, bsn, exported packages of the bundle)
    for bsn in sorted(best):
        _, path, mf = best[bsn]
        exports = set(split_clauses(mf.get('Export-Package', '')))
        bcp = split_clauses(mf.get('Bundle-ClassPath', '')) or ['.']
        if '.' not in bcp:
            bcp = ['.'] + bcp
        z = zipfile.ZipFile(path)
        for e in bcp:
            if e == '.':
                entries.append((path, bsn, exports))
            elif e.endswith('.jar') and e in z.namelist():
                d = os.path.join(lib, bsn); os.makedirs(d, exist_ok=True)
                out = os.path.join(d, os.path.basename(e))
                if not os.path.exists(out):
                    with open(out, 'wb') as fh:
                        fh.write(z.read(e))
                entries.append((out, bsn, exports))
    # rule 4: demote entries holding bundle-private copies of classes exported by another bundle
    exported_by = collections.defaultdict(set)  # class -> bundles exporting it
    private = collections.defaultdict(set)      # entry path -> classes it holds privately
    for path, bsn, exports in entries:
        for c in classes(path):
            if pkg(c) in exports:
                exported_by[c].add(bsn)
            else:
                private[path].add(c)
    demoted = [e for e in entries if any(exported_by[c] - {e[1]} for c in private[e[0]])]
    order = [e for e in entries if e not in demoted] + demoted
    return [e[0] for e in order]


def owner_of(path):
    """Bundle symbolic name of a classpath entry (plugin jar: its manifest; nested jar: <lib>/<bsn>/x.jar)."""
    mf = manifest(path)
    if mf and 'Bundle-SymbolicName' in mf:
        return mf['Bundle-SymbolicName'].split(';')[0].strip(), set(split_clauses(mf.get('Export-Package', '')))
    bsn = os.path.basename(os.path.dirname(path))
    return bsn, None  # exports: taken from the owning bundle below


def check(cpfile, verbose):
    cp = [p for p in open(cpfile).read().strip().split(':') if p]
    info = {}
    bundle_exports = {}
    for p in cp:
        bsn, ex = owner_of(p)
        info[p] = bsn
        if ex is not None:
            bundle_exports.setdefault(bsn, ex)
    where = collections.defaultdict(list)
    for p in cp:
        for c in classes(p):
            where[c].append(p)
    groups = collections.defaultdict(list)  # (jars, exporting jars) -> classes
    for c, ps in where.items():
        if len(ps) > 1:
            exp = tuple(p for p in ps if pkg(c) in bundle_exports.get(info[p], set()))
            groups[(tuple(ps), exp)].append(c)
    bad = 0
    for (ps, exporters), cs in sorted(groups.items()):
        pk = sorted({pkg(c) for c in cs})
        if len({info[p] for p in ps}) == 1:
            verdict, ok = 'same bundle %s: Bundle-ClassPath order' % info[ps[0]], True
        elif len({info[p] for p in exporters}) == 1:
            ok = info[ps[0]] == info[exporters[0]]
            verdict = 'exported only by %s -> %s' % (info[exporters[0]], 'first: OK' if ok else 'SHADOWED by a private copy')
        else:
            rev = [r for pre, r in REVIEWED.items() if all(c.startswith(pre) for c in cs)]
            ok = bool(rev)
            kind = 'several exporters' if exporters else 'private in every bundle'
            verdict = kind + (', reviewed: ' + rev[0] if ok else ': NOT REVIEWED')
        bad += not ok
        print('%s %d classes in %d packages (%s%s)' % ('ok  ' if ok else 'FAIL', len(cs), len(pk), pk[0], ', ...' if len(pk) > 1 else ''))
        for p in ps:
            print('       %s  [%s]' % (os.path.basename(p), info[p]))
        print('       ' + verdict)
        if verbose:
            for c in sorted(cs):
                print('         ' + c)
    print('%d duplicate sets, %d problems' % (len(groups), bad))
    return 1 if bad else 0


if __name__ == '__main__':
    a = sys.argv[1:]
    if len(a) == 3 and a[0] == 'gen':
        print(':'.join(gen(a[1], a[2])))
    elif len(a) >= 2 and a[0] == 'check':
        sys.exit(check(a[1], '-v' in a))
    else:
        sys.exit(__doc__)
