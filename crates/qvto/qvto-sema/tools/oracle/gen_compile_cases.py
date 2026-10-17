#!/usr/bin/env python3
"""Writes qvto-sema/tests/data/compile.cases: one compile-only case per corpus file of the
parse oracle (qvto-syntax/tests/fixtures/eclipse-parse-oracle.tsv), for Eclipse QVTo's
full-compile oracle (make-compile-fixture.sh). Run from anywhere with `python3 -I`.

Environment per file (rule in the case's `note env` line):
  case      an Eclipse file that is the transformation of a runnable case of
            qvto-testkit/cases/eclipse.cases: that case's compile environment (the first
            such case in file order); TestQvtParser cases keep their `check problems`
  sibling   an Eclipse file below the directory of such a case (an imported unit): the
            environment of the case of the deepest such directory
  sources   other files under parserTestData/sources/<dir>/: TestQvtParser's environment
  default   other Eclipse files: the bundles' source containers, bundle-blackboxes; if the file's
            imports resolve better against one of its ancestor directories (deployed/,
            parserTestData/externlib/: the roots PlatformPluginUnitResolver gets in the suite), that
            directory as `imports root`
  simulizar SimuLizar: the PCM configuration, imports from the bundle (its qvtTransformationContainer)
  spd       SPD: the PCM configuration, imports from the bundle's source container
plus the per-file additions of EXTRA (what a driver sets up in Java). SimuLizar's and SPD's Java
blackboxes are stubs (tools/oracle/stubs, on the runner's classpath) registered under the units'
names (`blackbox` lines, as their plugin.xml declares them); the Java libraries of the JDT tests
(bug578265, bug579914) are compiled onto the classpath and resolve by class name.

Paths are relative to the QVTo root (crates/qvto); SimuLizar and SPD files keep their original location
(../../../old/palladio-research/repos/... relative to crates/qvto, as tools/oracle/corpus-files.sh of qvto-syntax). Compile
only: every case has a `check problems` (TestQvtParser's counts, else `0 -1`: passes iff Eclipse
reports no errors).
"""
import os
import re
import sys
from collections import OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, '..', '..', '..', '..'))
PARSE_FIXTURE = os.path.join(ROOT, 'qvto-syntax/tests/fixtures/eclipse-parse-oracle.tsv')
ECLIPSE_CASES = os.path.join(ROOT, 'qvto-testkit/cases/eclipse.cases')
OUT = os.path.join(ROOT, 'qvto-sema/tests/data/compile.cases')

ECLIPSE = 'reference/eclipse-qvto'
REPOS = '../../../old/palladio-research/repos'
SIMULIZAR = REPOS + '/Palladio-Analyzer-SimuLizar'
SPD = REPOS + '/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations'
ORIGINS = {'eclipse': ECLIPSE, 'simulizar': SIMULIZAR, 'spd': SPD}
PCM_CONF = 'target/pcm/pcm.conf'

# keys of a case that matter for compiling (the rest is about running)
COMPILE_KEYS = ('mmfile', 'package', 'blackbox', 'blackbox-modules', 'bundle-blackboxes', 'no-line-numbers',
                'imports', 'urimap', 'conf')
ECORE = 'http://www.eclipse.org/emf/2002/Ecore'
# Java blackboxes of SimuLizar and SPD: stub class, unit, module, metamodels
STUB_BLACKBOXES = {
    'simulizar': [
        ('qvtosema.stubs.SimulizarProfilesLibrary', 'org.palladiosimulator.simulizar.action.repository.black.ProfilesLibrary',
         'ProfilesLibrary', ECORE),
        ('qvtosema.stubs.SimulationStateLibrary', 'org.palladiosimulator.simulizar.action.repository.black.SimulationStateLibrary',
         'SimulationStateLibrary', ECORE),
    ],
    'spd': [
        ('qvtosema.stubs.ArchitecturalTemplatesProfilesLibrary',
         'org.palladiosimulator.architecturaltemplates.catalog.black.ProfilesLibrary', 'ProfilesLibrary', ECORE),
    ],
}
# per-file additions (key -> case lines; paths relative to the file's folder)
EXTRA = {
    # DynamicModelInvocationTest: TransformationExecutor with a registry holding the loaded package
    'eclipse:tests/org.eclipse.m2m.tests.qvt.oml/deployed/dynamicmodel/nonregisteredDynamic.qvto':
        [('mmfile', 'nonregistered.ecore')],
}
PARSER_ENV = ['bundle-blackboxes', 'imports folder',
              f'urimap platform:/resource/ParserTest/ {ECLIPSE}/tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/']


def read_keys():
    keys = []
    seen = set()
    with open(PARSE_FIXTURE, encoding='utf-8') as f:
        for line in f:
            k = line.split('\t', 1)[0]
            if k and k not in seen:
                seen.add(k)
                keys.append(k)
    return sorted(keys)


def read_cases(path):
    """Runnable cases: (id, [(key, value)]) in file order."""
    out = []
    cur = None
    with open(path, encoding='utf-8') as f:
        for raw in f:
            line = raw.strip()
            if not line or line.startswith('#'):
                continue
            k, _, v = line.partition(' ')
            if k == 'case':
                cur = (v, [])
            elif cur is None:
                continue
            elif k == 'end':
                if not any(x == 'unrunnable' for x, _ in cur[1]) and any(x == 'qvto' for x, _ in cur[1]):
                    out.append(cur)
                cur = None
            else:
                cur[1].append((k, v))
    return out


def case_id(key):
    origin, _, rel = key.partition(':')
    rel = rel.replace('tests/org.eclipse.m2m.tests.qvt.oml/', 'oml/', 1) if origin == 'eclipse' else rel
    if rel.endswith('.qvto'):
        rel = rel[:-len('.qvto')]
    return origin + '__' + re.sub(r'[^A-Za-z0-9_.-]', '_', rel.replace('/', '__'))


def plugin_container(bundle):
    """First qvtTransformationContainer source container of a bundle's plugin.xml ('' if none)."""
    px = os.path.join(ROOT, bundle, 'plugin.xml')
    if not os.path.isfile(px):
        return ''
    text = open(px, encoding='utf-8').read()
    text = re.sub(r'<!--.*?-->', '', text, flags=re.S)
    m = re.search(r'qvtTransformationContainer.*?<sourceContainer\s+path="([^"]*)"', text, re.S)
    return m.group(1).strip('/') if m else ''


IMPORT = re.compile(r'(?m)^\s*import\s+(?:library\s+)?([\w.]+)\s*;')


def unit_imports(path):
    text = re.sub(r'/\*.*?\*/', '', open(os.path.join(ROOT, path), encoding='utf-8').read(), flags=re.S)
    text = re.sub(r'--[^\n]*', '', text)
    return IMPORT.findall(text)


def resolved(imports, folder, bases):
    """How many imports resolve to a .qvto file against the bases, a simple name also against the
    importing file's folder (QVTOCompiler.resolveImportedUnit: the importer's namespace)."""
    n = 0
    for qn in imports:
        rel = qn.replace('.', '/') + '.qvto'
        if any(os.path.isfile(os.path.join(ROOT, b, rel)) for b in bases + ([folder] if '.' not in qn else [])):
            n += 1
    return n


def import_root(path):
    """An ancestor directory of an Eclipse file (below its bundle) that resolves more of its imports
    than the bundle's source containers, or None."""
    imports = unit_imports(path)
    if not imports:
        return None
    parts = path.split('/')
    bundle = '/'.join(parts[:4])    # reference/eclipse-qvto/<tests|plugins>/<bundle>
    folder = os.path.dirname(path)
    bases = []
    if parts[2] == 'tests':
        bases = [bundle + ('/' + c if c else '') for c in containers(bundle)]
    best, best_n = None, resolved(imports, folder, bases)
    d = folder
    while d != bundle:
        n = resolved(imports, folder, [d])
        if n > best_n:
            best, best_n = d, n
        d = os.path.dirname(d)
    return best


def containers(bundle):
    """The qvtTransformationContainer source containers of a bundle's plugin.xml (as
    reference/runner's BundleUnitResolverFactory reads them; [''] if none)."""
    px = os.path.join(ROOT, bundle, 'plugin.xml')
    out = []
    if os.path.isfile(px):
        text = re.sub(r'<!--.*?-->', '', open(px, encoding='utf-8').read(), flags=re.S)
        for m in re.finditer(r'<sourceContainer\s+path="([^"]*)"', text):
            p = m.group(1).strip()
            if p:
                out.append('' if p == '/' else p.strip('/'))
    return out or ['']


def bundle_of(path):
    """Nearest ancestor directory of a file (relative to ROOT) that is an Eclipse bundle or project
    (not a repository's bundles/ or tests/ folder); else the file's folder."""
    d = os.path.dirname(path)
    while d and d not in ('.', '..') and not d.endswith(('/bundles', '/tests')):
        if os.path.isfile(os.path.join(ROOT, d, 'META-INF/MANIFEST.MF')) or os.path.isfile(os.path.join(ROOT, d, '.project')):
            return d
        d = os.path.dirname(d)
    return os.path.dirname(path)


def main():
    keys = read_keys()
    cases = read_cases(ECLIPSE_CASES)
    by_file = OrderedDict()   # path of the transformation (relative to ROOT) -> first case
    conflicts = []
    for cid, lines in cases:
        d = next(v for k, v in lines if k == 'dir')
        q = next(v for k, v in lines if k == 'qvto')
        p = os.path.normpath(os.path.join(d, q))
        env = [(k, v) for k, v in lines if k in COMPILE_KEYS]
        if p in by_file:
            if [x for x in by_file[p][2] if x[0] != 'mmfile'] != [x for x in env if x[0] != 'mmfile'] \
                    or by_file[p][1] != d:
                conflicts.append((p, by_file[p][0], cid))
            continue
        by_file[p] = (cid, d, env, [v for k, v in lines if k == 'check' and v.startswith('problems ')])
    case_dirs = sorted({v[1]: v for v in by_file.values()}.values(), key=lambda v: -len(v[1]))

    out = ['# Generated by qvto-sema/tools/oracle/gen_compile_cases.py: one compile-only case per file of',
           '# the parse oracle (qvto-syntax/tests/fixtures/eclipse-parse-oracle.tsv) for Eclipse QVTo\'s',
           '# full-compile oracle (tools/oracle/make-compile-fixture.sh). Paths are relative to the QVTo root (crates/qvto).',
           '# Format: qvto-testkit/README.md. `note key` is the oracle key, `note env` the environment rule.',
           '']
    ids = set()
    tally = {}
    for key in keys:
        origin, _, rel = key.partition(':')
        path = os.path.normpath(ORIGINS[origin] + '/' + rel)
        if not os.path.isfile(os.path.join(ROOT, path)):
            sys.exit(f'missing corpus file {path}')
        cid = case_id(key)
        if cid in ids:
            sys.exit(f'duplicate case id {cid}')
        ids.add(cid)
        env, check, rule, src = [], ['problems 0 -1'], None, None
        if origin == 'eclipse':
            if path in by_file:
                src, d, env, checks = by_file[path]
                rule = 'case'
                check = checks or check
            else:
                owner = next((v for v in case_dirs if path.startswith(v[1] + '/')), None)
                if owner is not None:
                    src, d, env, _ = owner
                    rule = 'sibling'
                elif '/parserTestData/sources/' in path:
                    m = re.match(r'(.*/parserTestData/sources/[^/]+)/', path)
                    d = m.group(1)
                    env = [tuple(x.split(' ', 1)) if ' ' in x else (x, '') for x in PARSER_ENV]
                    rule = 'sources'
                else:
                    d = os.path.dirname(path)
                    env = [('bundle-blackboxes', '')]
                    rule = 'default'
                    root = import_root(path)
                    if root:
                        env.append(('imports', 'root ' + root))
            qvto = os.path.relpath(path, d)
        else:
            d = os.path.dirname(path)
            qvto = os.path.basename(path)
            bundle = bundle_of(path)
            c = plugin_container(bundle)
            root = bundle + ('/' + c if c else '')
            if not path.startswith(root + '/'):
                root = bundle
            env = [('conf', PCM_CONF), ('imports', 'root ' + root)]
            env += [('blackbox', ' '.join(b)) for b in STUB_BLACKBOXES[origin]]
            rule = origin
        if key in EXTRA:
            env = env + EXTRA[key]
            rule += '+extra'
        tally[rule] = tally.get(rule, 0) + 1
        out.append(f'case {cid}')
        out.append('driver compile-oracle')
        out.append('kind compile')
        out.append(f'dir {d}')
        out.append(f'qvto {qvto}')
        for k, v in env:
            out.append(f'{k} {v}'.rstrip())
        out.append(f'note key {key}')
        out.append(f'note env {rule}' + (f' from {src}' if src else ''))
        for c in check:
            out.append(f'check {c}')
        out.append('end')
        out.append('')
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, 'w', encoding='utf-8') as f:
        f.write('\n'.join(out))
    print(f'{len(keys)} cases written to {os.path.relpath(OUT, ROOT)}: ' +
          ', '.join(f'{k} {v}' for k, v in sorted(tally.items())))
    for p, a, b in conflicts:
        print(f'note: {p}: case {b} has another compile environment than {a} (used)', file=sys.stderr)


if __name__ == '__main__':
    main()
