#!/usr/bin/env python3
"""Generate qvto-testkit/cases/eclipse.cases from the Eclipse QVTo test suite.

Usage (from the QVTo root (crates/qvto)):
    python3 -I qvto-testkit/tools/gen_cases.py reference/eclipse-qvto \
        reference/inventory/eclipse-tests.tsv qvto-testkit/cases/eclipse.cases

One case per row of the inventory (523). Execution cases are read from the drivers' Java
sources (TransformTests.createTestData, TransformManyModelTests, TestFailedTransformation; the
anonymous subclasses' registerModule / getUsedPackages / getTestDataFolder / getExpectedSeverity
overrides and includeMetamodelFile calls); the few hand-written drivers (stack traces, config
properties, traces, ...) are encoded below with a reference to their source. Parser cases come
from TestQvtParser's TestData entries. Everything else is classified as not runnable outside
Eclipse, with the reason. The case-file format is documented in qvto-testkit/README.md.
"""
import csv
import os
import re
import sys
from collections import OrderedDict

ROOT = sys.argv[1]
INVENTORY = sys.argv[2]
OUT = sys.argv[3]
BUNDLE = 'tests/org.eclipse.m2m.tests.qvt.oml'
REF = os.path.relpath(ROOT, os.getcwd())          # path of the checkout relative to the repo root
SRC = os.path.join(ROOT, BUNDLE, 'src/org/eclipse/m2m/tests/qvt/oml')


def rd(p):
    with open(p, encoding='utf-8', errors='replace') as f:
        return f.read()


# ----------------------------------------------------------------- Java parsing
def strip_java_comments(s):
    out, i, n = [], 0, len(s)
    while i < n:
        c = s[i]
        if c == '"' or c == "'":
            j = i + 1
            while j < n and s[j] != c:
                if s[j] == '\\':
                    j += 1
                j += 1
            out.append(s[i:j + 1])
            i = j + 1
        elif s.startswith('//', i):
            j = s.find('\n', i)
            i = n if j < 0 else j
        elif s.startswith('/*', i):
            j = s.find('*/', i + 2)
            i = n if j < 0 else j + 2
        else:
            out.append(c)
            i += 1
    return ''.join(out)


def balanced(s, i, open_c, close_c):
    depth, n = 0, len(s)
    while i < n:
        c = s[i]
        if c == '"':
            j = i + 1
            while s[j] != '"':
                if s[j] == '\\':
                    j += 1
                j += 1
            i = j + 1
            continue
        if c == open_c:
            depth += 1
        elif c == close_c:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise ValueError('unbalanced')


def split_top(args):
    parts, depth, cur, i = [], 0, [], 0
    while i < len(args):
        c = args[i]
        if c == '"':
            j = i + 1
            while args[j] != '"':
                if args[j] == '\\':
                    j += 1
                j += 1
            cur.append(args[i:j + 1])
            i = j + 1
            continue
        if c in '([{<':
            depth += 1
        elif c in ')]}>':
            depth -= 1
        if c == ',' and depth == 0:
            parts.append(''.join(cur).strip())
            cur = []
        else:
            cur.append(c)
        i += 1
    if ''.join(cur).strip():
        parts.append(''.join(cur).strip())
    return parts


STR = re.compile(r'"((?:[^"\\]|\\.)*)"')


def unescape(s):
    return re.sub(r'\\(.)', lambda m: {'n': '\n', 't': '\t'}.get(m.group(1), m.group(1)), s)


def parse_arg(a):
    a = a.strip()
    if a.startswith('"'):
        return ('str', unescape(STR.match(a).group(1)))
    if re.match(r'(Arrays\.asList|Collections\.singletonList)', a):
        return ('list', [unescape(x) for x in STR.findall(a)])
    if 'emptyList' in a:
        return ('list', [])
    if a.startswith('new String[][]'):
        body = a[a.index('{') + 1:a.rindex('}')]
        props = []
        for m in re.finditer(r'\{([^{}]*)\}', body):
            vals = split_top(m.group(1))
            k = unescape(STR.match(vals[0]).group(1))
            v = vals[1].strip()
            v = unescape(STR.match(v).group(1)) if v.startswith('"') else None
            props.append((k, v))
        return ('props', props)
    if a in ('true', 'false'):
        return ('bool', a == 'true')
    return ('other', a)


DATA_CLASSES = r'(FileToFileData|FilesToFilesData|ReferencedProjectData|PluginDependencyProjectData)'


def extract_entries(text):
    entries, pos = [], 0
    pat = re.compile(r'new\s+' + DATA_CLASSES + r'\s*\(')
    while True:
        m = pat.search(text, pos)
        if not m:
            break
        kind = m.group(1)
        p_open = m.end() - 1
        p_close = balanced(text, p_open, '(', ')')
        args = [parse_arg(a) for a in split_top(text[p_open + 1:p_close - 1])]
        j = p_close
        while j < len(text) and text[j].isspace():
            j += 1
        body = ''
        if j < len(text) and text[j] == '{':
            k = balanced(text, j, '{', '}')
            body = text[j:k]
            j = k
        mms = []
        while True:
            mm = re.match(r'\s*\.includeMetamodelFile\(\s*"([^"]*)"\s*\)', text[j:])
            if not mm:
                break
            mms.append(mm.group(1))
            j += mm.end()
        entries.append(make_entry(kind, args, body, mms))
        pos = j
    return entries


def make_entry(kind, args, body, mms):
    e = OrderedDict(kind=kind, name=args[0][1], inputs=[], expected=[], props=[], body=body, mms=list(mms),
                    cycle=None, referenced=None)
    rest = args[1:]
    if kind == 'FileToFileData':
        strs = [a[1] for a in rest if a[0] == 'str']
        e['inputs'] = [strs[0]] if len(strs) >= 2 else ['in.ecore']
        e['expected'] = [strs[1]] if len(strs) >= 2 else ['expected.ecore']
    elif kind == 'FilesToFilesData':
        lists = [a[1] for a in rest if a[0] == 'list']
        if len(lists) >= 2:
            e['inputs'], e['expected'] = lists[0], lists[1]
    else:
        e['referenced'] = rest[0][1]
        e['cycle'] = rest[1][1]
    for a in rest:
        if a[0] == 'props':
            e['props'] = a[1]
    # registerModule(X.class[, "unit", "module"[, new String[] {"ns", ...}]]) and registerModule(libraryClass)
    regs = []
    for m in re.finditer(r'registerModule\(', body):
        p_close = balanced(body, m.end() - 1, '(', ')')
        a = split_top(body[m.end():p_close - 1])
        cls = a[0]
        if cls.endswith('.class'):
            cls = cls[:-len('.class')]
        else:  # a variable: Class<?> libraryClass = X.class;
            mv = re.search(r'Class<\?>\s*' + re.escape(cls) + r'\s*=\s*([\w.]+)\.class', body)
            cls = mv.group(1)
        unit = STR.match(a[1]).group(1) if len(a) > 1 else None
        mod = STR.match(a[2]).group(1) if len(a) > 2 else None
        nss = STR.findall(a[3]) if len(a) > 3 else []
        regs.append((cls, unit, mod, nss))
    e['blackboxes'] = regs
    e['used_packages'] = re.findall(r'packages\.add\(\s*(\w+)\.eINSTANCE', body)
    m = re.search(r'getTestDataFolder\(\)\s*\{\s*return\s+"([^"]*)"', body)
    e['data_folder'] = m.group(1) if m else 'parserTestData'
    m = re.search(r'getBundle\(\)\s*\{\s*return\s+"([^"]*)"', body)
    e['bundle'] = m.group(1) if m else 'org.eclipse.m2m.tests.qvt.oml'
    m = re.search(r'getExpectedSeverity\(\)\s*\{\s*return\s+Diagnostic\.(\w+)', body)
    e['severity'] = m.group(1) if m else 'OK'
    e['session'] = re.findall(r'QVTEvaluationOptions\.(\w+)\s*,\s*([\w.]+)', body)
    return e


def imports_of(text):
    """Simple class name -> fully qualified name, from the import lines."""
    return {m.group(2): m.group(1) + m.group(2) for m in re.finditer(r'(?m)^import\s+([\w.]+\.)(\w+)\s*;', text)}


def ns_uri_of_package_class(simple):
    """nsURI of a generated EPackage interface, from its eNS_URI constant in the sources."""
    for d, _, fs in os.walk(ROOT):
        if simple + '.java' in fs:
            t = rd(os.path.join(d, simple + '.java'))
            m = re.search(r'String\s+eNS_URI\s*=\s*"([^"]*)"', t)
            if m:
                return m.group(1)
    raise SystemExit('no eNS_URI for ' + simple)


SEVERITY = {'OK': 0, 'INFO': 1, 'WARNING': 2, 'ERROR': 4, 'CANCEL': 8}

# ----------------------------------------------------------------- inventory
inventory = OrderedDict()
with open(INVENTORY, encoding='utf-8') as f:
    for row in csv.DictReader(f, delimiter='\t', quoting=csv.QUOTE_NONE):
        inventory[row['test_id']] = row

cases = OrderedDict()   # test_id -> list of lines


def bundle_dir(bundle='org.eclipse.m2m.tests.qvt.oml'):
    return f'{REF}/tests/{bundle}'


def common(lines, row):
    lines.append(f'driver {row["driving_test_class"]}')
    lines.append(f'kind {row["expected_kind"]}')
    if row['feature_tags']:
        lines.append(f'features {row["feature_tags"]}')


def exec_case(test_id, e, imports, driver_extra=()):
    row = inventory[test_id]
    folder = f'{bundle_dir(e["bundle"])}/{e["data_folder"]}/models/{e["name"]}'
    lines = []
    common(lines, row)
    lines.append(f'dir {folder}')
    lines.append(f'qvto {e["name"]}.qvto')
    for i in e['inputs']:
        lines.append(f'in {i}')
    for x in e['expected']:
        lines.append(f'expected {x}')
    for mm in e['mms']:
        lines.append(f'mmfile {mm}')
    for p in e['used_packages']:
        lines.append(f'package {ns_uri_of_package_class(p)}')
    for k, v in e['props']:
        lines.append(f'config-null {k}' if v is None else f'config {k}={v}')
    for cls, unit, mod, nss in e['blackboxes']:
        fq = imports.get(cls.split('.')[0], cls) if '.' not in cls else cls
        lines.append('blackbox ' + ' '.join(x for x in [fq, unit, mod, ','.join(nss)] if x))
    for opt, val in e['session']:
        if opt == 'FLAG_READONLY_GUARD_ENABLED':
            lines.append('session readonly-guard')
        elif opt == 'EVALUATION_MAX_STACK_DEPTH':
            lines.append(f'session max-stack-depth {val}')
        else:
            raise SystemExit('session option ' + opt)
    if os.path.exists(os.path.join(ROOT, os.path.relpath(folder, REF), e['name'] + '.qvtotrace')):
        lines.append(f'trace-in {e["name"]}.qvtotrace')
    lines.extend(driver_extra)
    cases[test_id] = lines


# --- TransformTests (run as TestQvtStandaloneExecutor runs them) -------------------------------
tt = rd(os.path.join(SRC, 'transform/TransformTests.java'))
tt_imports = imports_of(tt)
body = strip_java_comments(tt)
seen = {}
for e in extract_entries(body):
    name = e['name']
    if e['kind'] in ('ReferencedProjectData', 'PluginDependencyProjectData'):
        test_id = f'interpret_{name}_{"cycle" if e["cycle"] else "nocycle"}'
    else:
        test_id = f'interpret_{name}'
    assert test_id in inventory, test_id
    assert test_id not in seen, test_id
    seen[test_id] = True
    exec_case(test_id, e, tt_imports, [
        'source tests/org.eclipse.m2m.tests.qvt.oml/src/org/eclipse/m2m/tests/qvt/oml/transform/TransformTests.java',
        'emulates callapi.TestQvtStandaloneExecutor',
        f'check severity {SEVERITY[e["severity"]]} 0',
        'check model',
    ])
    if e['kind'] in ('ReferencedProjectData', 'PluginDependencyProjectData'):
        cases[test_id].insert(0, f'note project reference to {e["referenced"]} (cycle={e["cycle"]}) is a workspace '
                              'setting; imports resolve from the source container here')

# --- TransformManyModelTests (TestQvtInterpreter: outputs compared with the expected models) ----
mm = strip_java_comments(rd(os.path.join(SRC, 'transform/TransformManyModelTests.java')))
for e in extract_entries(mm):
    exec_case(f'manymodel_{e["name"]}', e, {}, [
        'source tests/org.eclipse.m2m.tests.qvt.oml/src/org/eclipse/m2m/tests/qvt/oml/transform/TransformManyModelTests.java',
        'emulates transform.TestQvtInterpreter (TransformationChecker) through the standalone executor',
        'bundle-blackboxes',
        'check severity 0 0',
        'check model',
    ])

# --- TestFailedTransformation (log must equal qvtTraceLog.stack) --------------------------------
ft = strip_java_comments(rd(os.path.join(SRC, 'transform/TestFailedTransformation.java')))
for e in extract_entries(ft):
    exec_case(f'failed_{e["name"]}', e, {}, [
        'source tests/org.eclipse.m2m.tests.qvt.oml/src/org/eclipse/m2m/tests/qvt/oml/transform/TestFailedTransformation.java',
        'bundle-blackboxes',
        'check log-nonempty-equal qvtTraceLog.stack',
    ])
    # the driver checks only the log; the expected outputs of FileToFileData are not compared
    cases[f'failed_{e["name"]}'] = [l for l in cases[f'failed_{e["name"]}'] if not l.startswith('expected ')]


# --- hand-written drivers -------------------------------------------------------------------
def manual(test_id, folder, qvto, ins, extra, source):
    row = inventory[test_id]
    lines = []
    common(lines, row)
    lines.append(f'dir {bundle_dir()}/{folder}')
    lines.append(f'qvto {qvto}')
    for i in ins:
        lines.append(f'in {i}')
    lines.append(f'source tests/org.eclipse.m2m.tests.qvt.oml/src/org/eclipse/m2m/tests/qvt/oml/{source}')
    lines.extend(extra)
    cases[test_id] = lines


ST = 'transform/TestStackTrace.java'
st_common = ['bundle-blackboxes']
manual('stacktrace_testMappingCallInProperty', 'parserTestData/models/stacktrace', 'stacktrace.qvto', ['in.ecore'],
       st_common + ['config testcase=mappingCallInProperty', 'check stack mappingCallInProperty.stack'], ST)
manual('stacktrace_testLogExpUsage', 'parserTestData/models/stacktrace', 'stacktrace.qvto', ['in.ecore'],
       st_common + ['config testcase=testLogExpUsage', 'check log-nonempty-equal testLogExpUsage.log.txt'], ST)
manual('stacktrace_testAssertionFailed', 'parserTestData/models/stacktrace', 'stacktrace.qvto', ['in.ecore'],
       st_common + ['config testcase=assertionFailed',
                    'check exception org.eclipse.m2m.internal.qvt.oml.evaluator.QvtAssertionFailed',
                    'check stack assertionFailed.stack', 'check log-nonempty-equal assertionFailed.log.txt'], ST)
manual('stacktrace_testStackOverFlow', 'parserTestData/models/stacktrace', 'stacktrace.qvto', ['in.ecore'],
       st_common + ['config testcase=stackOverFlow',
                    'check exception org.eclipse.m2m.internal.qvt.oml.evaluator.QvtStackOverFlowError',
                    'check stack-frames 10 causeStackOverFlow auxtransf auxtransf.qvto 5|6'], ST)
manual('stacktrace_testCreateInstaceFailure', 'parserTestData/models/stacktrace', 'stacktrace.qvto', ['in.ecore'],
       st_common + ['config testcase=createInstaceFailure', 'check stack createInstaceFailure.stack'], ST)
manual('stacktrace_testUknownSourceStackTrace', 'parserTestData/models/stacktrace', 'stacktrace.qvto', ['in.ecore'],
       st_common + ['config testcase=createInstaceFailure', 'no-line-numbers',
                    'check stack testUknownSourceStackTrace.stack'], ST)
IC = 'transform/TestInvalidConfigProperty.java'
manual('invalidConfigProp_testInvalidConfigProp', 'parserTestData/models/invalidConfigProp', 'invalidConfigProp.qvto',
       ['in.ecore'], st_common + ['config intProp=xxx', 'config testcase=invalidConfigProp',
                                  'check stack invalidIntConfigProp.stack'], IC)
manual('invalidConfigProp_testInvalidRealConfigProp', 'parserTestData/models/invalidConfigProp', 'invalidConfigProp.qvto',
       ['in.ecore'], st_common + ['config realProp=xxx', 'config testcase=invalidConfigProp',
                                  'check stack invalidRealConfigProp.stack'], IC)
manual('invalidConfigProp_testUndefinedConfigProp', 'parserTestData/models/invalidConfigProp', 'invalidConfigProp.qvto',
       ['in.ecore'], st_common + ['config testcase=invalidConfigProp', 'check no-exception'], IC)
manual('blackboxlib_context_testInvalidConfigProp', 'parserTestData/models/blackboxlib_context', 'blackboxlib_context.qvto',
       ['in.ecore'], st_common + ['config strTest=test', 'config strTest2=test2', 'config testcase=executionTerminated',
                                  'check exception org.eclipse.m2m.internal.qvt.oml.evaluator.QvtInterruptedExecutionException',
                                  'check stack executionTerminated.stack',
                                  'check log-nonempty-equal executionTerminated.log.txt'],
       'transform/TestBlackboxLibContext.java')
for name, n in (('multipletracerecords', 5), ('mappingWithNoResultTrace_266854', 1)):
    manual(f'trace_{name}', f'parserTestData/models/{name}', f'{name}.qvto', ['in.ecore'],
           st_common + [f'check trace-records {n}'], 'traces/TestTraceFile.java')
for cyc in ('nocycle', 'cycle'):
    manual(f'incorrect_bug433937_wrongImport_{cyc}', 'parserTestData/models/bug433937_wrongImport',
           'bug433937_wrongImport.qvto', [],
           ['note the driver expects a build error marker; here: the transformation fails to load',
            'check load-error'], 'transform/TestIncorrectTransformation.java')
for name in ('bug326871_standalone', 'bug326871a_standalone'):
    manual(f'callapi_{name}', f'parserTestData/models/{name}', f'{name}.qvto', [],
           ['blackbox-modules org.eclipse.m2m.tests.qvt.oml.bbox.StandaloneBBoxModuleRegistered', 'check severity 0 0'],
           'callapi/BlackboxModuleStandaloneSetupTest.java')
API = 'transform/api/ExecTransformationTestCase.java'
for name in ('exec1', 'exec2'):
    manual(f'api_{name}', f'apiTestData/{name}', f'{name}.qvto', ['in1.ecore', 'in2.ecore'],
           ['expected expected.simpleuml', 'expected expected.rdb', 'bundle-blackboxes',
            'note emulated through the standalone executor (the driver uses the QvtTransformation API)',
            'check severity 0 0', 'check model'], API)
manual('api_exec3', 'apiTestData/exec3', 'exec3.qvto', [],
       ['bundle-blackboxes', 'note emulated through the standalone executor', 'check severity 0 0'], API)
manual('api_exec3_withImport', 'apiTestData/exec3_withImport', 'exec3_withImport.qvto', [],
       ['bundle-blackboxes', 'note emulated through the standalone executor', 'check severity 0 0'],
       'transform/api/ExecDeployedTransformationTestCase.java')

# --- parser cases: compile only, error and warning counts ------------------------------------
pt = strip_java_comments(rd(os.path.join(SRC, 'TestQvtParser.java')))
for m in re.finditer(r'(?:new TestData|TestData\.createSourceChecked)\(\s*"([^"]+)"\s*,\s*(-?\d+)(?:\s*,\s*(-?\d+))?\s*\)'
                     r'((?:\s*\.includeMetamodel\(\s*"[^"]*"\s*\))*)', pt):
    name, err, warn, inc = m.group(1), m.group(2), m.group(3) or '-1', m.group(4)
    test_id = f'parse_{name}'
    row = inventory[test_id]
    lines = []
    common(lines, row)
    lines.append(f'dir {bundle_dir()}/parserTestData/sources/{name}')
    lines.append(f'qvto {name}.qvto')
    for mmf in re.findall(r'"([^"]*)"', inc or ''):
        lines.append(f'mmfile {mmf}')
    lines.append('source tests/org.eclipse.m2m.tests.qvt.oml/src/org/eclipse/m2m/tests/qvt/oml/TestQvtParser.java')
    lines.append('bundle-blackboxes')
    lines.append('imports folder')
    # TestQvtParser copies the folder into the workspace project "ParserTest" under sources/
    lines.append(f'urimap platform:/resource/ParserTest/ {bundle_dir()}/parserTestData/')
    cst = ' cst' if 'createSourceChecked' in m.group(0) else ''
    lines.append(f'check problems {err} {warn}{cst}')
    cases[test_id] = lines

# cases of runnable drivers that still need Eclipse
NEEDS_ECLIPSE = {
    'parse_bug573449': ('jdt', 'the blackbox @Module(packageURIs="http://bug573449") needs the package that the '
                        'workspace Java project setup (TestUtil.prepareJava) registers'),
    'api_exec3_withImport': ('osgi', "emf.tools through OSGi's LegacyNativeLibraryProviderAdapter (this.currentTimeMillis() "
                             'from a library); the output depends on the time'),
}
for test_id, (cat, why) in NEEDS_ECLIPSE.items():
    lines = [l for l in cases[test_id] if l.split(' ')[0] in ('driver', 'kind', 'features')]
    lines.append(f'unrunnable {cat} {why}')
    cases[test_id] = lines

# --- everything else: not runnable outside Eclipse --------------------------------------------
UNRUNNABLE = {
    'transform.api.InputExtensionsTestCase': ('api', 'checks the input model extensions QvtoTransformationHelper derives; no execution'),
    'transform.api.WrappedExecTransformationTestCase': ('api', 'asserts on the QvtTransformation wrapper API'),
    'transform.api.ImportedTransfTestCase': ('api', 'checks the resolved import set of the QvtModule API, not models'),
    'transform.api.InoutValidationTestCase': ('workspace', 'validates inout model URIs against workspace and platform:/plugin resources'),
    'callapi.InvocationTest': ('api', 'in-memory models built in Java; asserts on ExecutionDiagnostic objects'),
    'callapi.InvocationExtTest': ('api', 'in-memory models built in Java; asserts on extents programmatically'),
    'callapi.ChildInTreeInputInvocationTest': ('api', 'input extent built in Java from a non-root object'),
    'callapi.DynamicModelInvocationTest': ('api', 'asserts on dynamic EMF objects programmatically'),
    'callapi.DynamicRegisteredModelInvocationTest': ('api', 'asserts on dynamic EMF objects programmatically'),
    'callapi.TransformationExecutorWithVisitorDecoratorTest': ('api', 'installs a Java evaluation-visitor decorator'),
    'callapi.IncrementalUpdateExecutorTest': ('api', 'two passes with a Java-built trace; asserts object identity across passes'),
    'callapi.SourceContainerTests': ('pde', 'deployed-transformation resolution across plugin projects'),
    'IntermediatePropertyCleanupTest': ('api', 'asserts on Java-side adapters after execution'),
    'TestExternHelperCall': ('api', 'calls QVTo helpers from Java'),
    'perf.ExecutionTimeGuardTest': ('perf', 'wall-clock limits; covered by the benchmarks'),
    'ant.TestQvtAntScript': ('ant', 'driven by an Ant script'),
    'TestQvtBuilder': ('workspace', 'workspace builder and problem markers'),
    'Bug577992': ('jdt', 'compiles a Java project in the workspace (JDT/PDE)'),
    'Bug578265': ('jdt', 'compiles a Java project in the workspace (JDT/PDE)'),
    'Bug579914': ('jdt', 'compiles a Java project in the workspace (JDT/PDE)'),
    'QvtLibraryASTTest': ('api', 'inspects the AST of a Java blackbox library'),
}
for test_id, row in inventory.items():
    if test_id in cases:
        continue
    drv = row['driving_test_class']
    if drv not in UNRUNNABLE:
        raise SystemExit(f'unclassified case {test_id} ({drv})')
    cat, why = UNRUNNABLE[drv]
    lines = []
    common(lines, row)
    lines.append(f'unrunnable {cat} {why}')
    cases[test_id] = lines

missing = [t for t in cases if t not in inventory]
assert not missing, missing

with open(OUT, 'w', encoding='utf-8') as f:
    f.write('# Generated by qvto-testkit/tools/gen_cases.py from the Eclipse QVTo test suite\n')
    f.write('# (reference/eclipse-qvto, see reference/README.md) and reference/inventory/eclipse-tests.tsv.\n')
    f.write('# Paths are relative to the QVTo root (crates/qvto). Format: qvto-testkit/README.md.\n')
    for test_id in inventory:
        f.write(f'\ncase {test_id}\n')
        for l in cases[test_id]:
            f.write(l + '\n')
        f.write('end\n')
print(f'{len(cases)} cases written to {OUT}')
