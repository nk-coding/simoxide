#!/usr/bin/env python3
"""Build a machine-readable inventory of the Eclipse QVTo test suite.

Usage: python3 -I build_inventory.py <clone-root> <out.tsv>

Read-only on the clone. Heuristics are documented inline (search for HEURISTIC).
"""
import os
import re
import sys
from collections import Counter, OrderedDict

ROOT = os.path.abspath(sys.argv[1])
OUT = sys.argv[2]
BUNDLE = 'tests/org.eclipse.m2m.tests.qvt.oml'
SRC = os.path.join(ROOT, BUNDLE, 'src/org/eclipse/m2m/tests/qvt/oml')


def rd(p):
    with open(p, encoding='utf-8', errors='replace') as f:
        return f.read()


# ----------------------------------------------------------------- Java parsing
def strip_java_comments(s):
    out = []
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if c == '"' or c == "'":
            q = c
            j = i + 1
            while j < n and s[j] != q:
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
    """s[i] == open_c; return index just after the matching close."""
    depth = 0
    n = len(s)
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


def parse_arg(a):
    a = a.strip()
    if a.startswith('"'):
        return ('str', STR.match(a).group(1))
    if re.match(r'(Arrays\.asList|Collections\.singletonList)', a):
        return ('list', STR.findall(a))
    if 'emptyList' in a:
        return ('list', [])
    if a.startswith('new String[][]'):
        body = a[a.index('{') + 1:a.rindex('}')]
        props = []
        for m in re.finditer(r'\{([^{}]*)\}', body):
            vals = split_top(m.group(1))
            k = STR.match(vals[0]).group(1)
            v = vals[1].strip()
            v = STR.match(v).group(1) if v.startswith('"') else v
            props.append((k, v))
        return ('props', props)
    if a in ('true', 'false'):
        return ('bool', a == 'true')
    return ('other', a)


DATA_CLASSES = r'(FileToFileData|FilesToFilesData|ReferencedProjectData|PluginDependencyProjectData|ApiTestData)'


def extract_entries(text):
    """Extract `new XData(...) [{...}] [.includeMetamodelFile(..)]*` entries."""
    entries = []
    pos = 0
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
    e = OrderedDict(kind=kind, name=args[0][1], inputs=[], expected=[], props=[],
                    body=body, mms=list(mms), cycle=None, referenced=None)
    rest = args[1:]
    if kind == 'FileToFileData':
        strs = [a[1] for a in rest if a[0] == 'str']
        e['inputs'] = [strs[0]] if len(strs) >= 2 else ['in.ecore']
        e['expected'] = [strs[1]] if len(strs) >= 2 else ['expected.ecore']
    elif kind in ('FilesToFilesData', 'ApiTestData'):
        lists = [a[1] for a in rest if a[0] == 'list']
        if len(lists) >= 2:
            e['inputs'], e['expected'] = lists[0], lists[1]
    elif kind in ('ReferencedProjectData', 'PluginDependencyProjectData'):
        e['referenced'] = rest[0][1]
        e['cycle'] = rest[1][1]
    for a in rest:
        if a[0] == 'props':
            e['props'] = a[1]
    # anonymous-subclass overrides
    e['bbox_classes'] = re.findall(r'registerModule\(\s*([\w.]+)\.class', body)
    e['bbox_classes'] += re.findall(r'Class<\?>\s*\w+\s*=\s*([\w.]+)\.class', body)
    e['used_packages'] = re.findall(r'packages\.add\(\s*(\w+)\.eINSTANCE', body)
    m = re.search(r'getTestDataFolder\(\)\s*\{\s*return\s+"([^"]*)"', body)
    e['data_folder'] = m.group(1) if m else 'parserTestData'
    m = re.search(r'getBundle\(\)\s*\{\s*return\s+"([^"]*)"', body)
    e['bundle'] = m.group(1) if m else 'org.eclipse.m2m.tests.qvt.oml'
    m = re.search(r'getExpectedSeverity\(\)\s*\{\s*return\s+([\w.]+)', body)
    e['severity'] = m.group(1) if m else None
    e['session_opts'] = re.findall(r'QVTEvaluationOptions\.(\w+)\s*,\s*([\w.]+)', body)
    e['compiled_xmi_off'] = 'isUseCompiledXmi' in body and 'return false' in body
    return e


def method_body(text, name):
    m = re.search(r'\b' + name + r'\s*\([^)]*\)[^{;]*\{', text)
    if not m:
        return ''
    s = m.end() - 1
    return text[s:balanced(text, s, '{', '}')]


# ----------------------------------------------------------------- QVTo scanning
def strip_qvto_comments(s):
    out = []
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if c in ('"', "'"):
            j = i + 1
            while j < n and s[j] != c and s[j] != '\n':
                if s[j] == '\\':
                    j += 1
                j += 1
            out.append(s[i:j + 1])
            i = j + 1
        elif s.startswith('--', i) or s.startswith('//', i):
            j = s.find('\n', i)
            i = n if j < 0 else j
        elif s.startswith('/*', i):
            j = s.find('*/', i + 2)
            i = n if j < 0 else j + 2
        else:
            out.append(c)
            i += 1
    return ''.join(out)


def strip_strings(s):
    return re.sub(r'"(?:[^"\\\n]|\\.)*"|\'(?:[^\'\\\n]|\\.)*\'', '""', s)


# HEURISTIC: keyword/regex based feature tagging over all .qvto files of a test
# folder (comments and string literal contents removed before matching).
FEATURES = [
    ('mapping', r'\bmapping\b'),
    ('mapping_disjuncts', r'\bdisjuncts\b'),
    ('mapping_inherits', r'\binherits\b'),
    ('mapping_merges', r'\bmerges\b'),
    ('mapping_abstract', r'\babstract\s+mapping\b'),
    ('mapping_when_guard', r'\bwhen\s*\{'),
    ('mapping_where', r'\bwhere\s*\{'),
    ('mapping_init_section', r'\binit\s*\{'),
    ('mapping_population_section', r'\bpopulation\s*\{'),
    ('mapping_end_section', r'\bend\s*\{'),
    ('mapping_inout_param', r'\bmapping\s+inout\b|\(\s*inout\b|,\s*inout\b'),
    ('multiple_results', r'\bmapping\b[^{;]*\)\s*:\s*\w[\w:]*\s*\w*\s*,\s*\w'),
    ('legacy_main_signature', r'\b(?:mapping|query|helper)\s+main\s*\(\s*\w'),
    ('blackbox_transformation', r'\bblackbox\s+transformation\b'),
    ('xmap', r'\bxmap\b'),
    ('map_on_collection', r'->\s*x?map\b'),
    ('resolve', r'\bresolve(?:one)?\s*\('),
    ('resolveIn', r'\bresolve(?:one)?In\s*\('),
    ('invresolve', r'\binvresolve(?:one)?(?:In)?\s*\('),
    ('late_resolve', r'\blate\s+(?:inv)?resolve'),
    ('deepclone', r'\bdeepclone\b'),
    ('clone', r'\bclone\s*\('),
    ('object_expr', r'\bobject\b'),
    ('instantiation_new', r'\bnew\s+[\w:]+\s*\('),
    ('constructor', r'\bconstructor\b'),
    ('intermediate_class', r'\bintermediate\s+class\b'),
    ('intermediate_property', r'\bintermediate\s+property\b'),
    ('module_property', r'(?m)^\s*property\b'),
    ('configuration_property', r'\bconfiguration\s+property\b'),
    ('library', r'(?m)^\s*library\b'),
    ('import', r'(?m)^\s*import\b'),
    ('access', r'\baccess\b'),
    ('extends', r'\bextends\b'),
    ('blackbox_decl', r'\bblackbox\b'),
    ('helper', r'\bhelper\b'),
    ('query', r'\bquery\b'),
    ('contextual_op', r'\b(?:query|helper|mapping)\s+(?:inout\s+)?[\w:]+::\w+\s*\('),
    ('while', r'\bwhile\s*\('),
    ('forEach', r'->\s*(?:forEach|forOne)\b'),
    ('compute', r'\bcompute\s*\('),
    ('switch', r'\bswitch\b'),
    ('break_continue', r'\b(?:break|continue)\s*;'),
    ('return', r'\breturn\b'),
    ('let', r'\blet\b'),
    ('try_except', r'\btry\s*\{|\bexcept\b'),
    ('raise', r'\braise\b'),
    ('assert', r'\bassert\b'),
    ('log', r'\blog\s*\('),
    ('dict', r'\bDict\s*[({]'),
    ('list', r'\bList\s*[({]'),
    ('tuple', r'\bTuple\s*[({]'),
    ('collection_literal', r'\b(?:Set|Sequence|Bag|OrderedSet)\s*\{'),
    ('ocl_iterators', r'->\s*(?:select|reject|collect|exists|forAll|any|one|isUnique|sortedBy|iterate|closure|collectNested)\s*\('),
    ('xcollect_xselect', r'->\s*(?:xcollect|xselect|selectOne|collectOne|collectselect|collectselectOne)\b'),
    ('shorthand_select_brackets', r'->\s*\['),
    ('shorthand_type_filter_#T', r'(?<!#)#(?!#)\s*[A-Za-z_]'),
    ('shorthand_type_filter_##T', r'##\s*[A-Za-z_]'),
    ('bracket_shorthand_select_or_type_filter', r'(?:\)|[A-Za-z_]\w*)\s*\[(?!\s*[\d*])'),
    ('collection_ops', r'->\s*(?:including|excluding|union|intersection|flatten|asSet|asSequence|asBag|asOrderedSet|asList|first|last|at|size|isEmpty|notEmpty|includes|excludes|includesAll|excludesAll|count|sum|append|prepend|insertAt|indexOf|subSequence|subOrderedSet|reverse|max|min|add|put|get|hasKey|keys|values|defined|containsValue|clear|remove|removeAt|removeFirst|removeLast|addAll)\s*\('),
    ('string_ops', r'\.\s*(?:startsWith|endsWith|firstToUpper|lastToUpper|toUpper|toLower|toUpperCase|toLowerCase|substringBefore|substringAfter|indexOf|lastIndexOf|replace|replaceAll|matchBoundedRegex|tokenize|trim|normalizeSpace|quotify|unquotify|isQuoted|equalsIgnoreCase|match|find|rfind|format|asInteger|asFloat|asBoolean|repeat|characters|addSuffixNumber|substring|concat|toInteger|toReal|toBoolean)\s*\('),
    ('objectsOfType', r'\bobjectsOf(?:Type|Kind)\s*\('),
    ('model_ops', r'\.\s*(?:objects|rootObjects|removeElement|createEmptyModel|copy)\s*\('),
    ('allInstances', r'\ballInstances\s*\('),
    ('element_ops', r'\.\s*(?:subobjects|allSubobjects|subobjectsOfType|allSubobjectsOfType|subobjectsOfKind|allSubobjectsOfKind|container|metaClassName|markedAs|stereotypedBy|stereotypedStrictlyBy|_localId|_globalId|oclMetaClass)\s*\('),
    ('ocl_type_ops', r'\boclAs(?:Type|Set)\b|\boclIs(?:KindOf|TypeOf|Undefined|Invalid)\b'),
    ('transformation_composition', r'\.\s*(?:transform|parallelTransform)\s*\(|\bnew\s+\w+\s*\([^)]*\)\s*\.\s*transform'),
    ('transformation_instance', r'\bTransformation\b'),
    ('inout_model_param', r'(?m)^\s*(?:blackbox\s+)?transformation\b[^;{]*\binout\b'),
    ('model_param_out', r'(?m)^\s*(?:blackbox\s+)?transformation\b[^;{]*\bout\b'),
    ('header_without_model_params', r'(?m)^\s*transformation\s+\w+\s*(?:\(\s*\))?\s*(?:;|\{|access|extends)'),
    ('modeltype_strict', r'\bstrict\b'),
    ('modeltype_where', r'\bmodeltype\b[^;]*\bwhere\b'),
    ('inline_metamodel', r'(?m)^\s*metamodel\b'),
    ('tag', r'(?m)^\s*tag\b'),
    ('rename', r'(?m)^\s*rename\b'),
    ('var_decl', r'\bvar\b'),
    ('this_self', r'\bthis\b'),
    ('static_op', r'\bstatic\b'),
    ('exceptions_decl', r'\bexception\b'),
    ('main_entry', r'\bmain\s*\('),
]
FEATURES = [(n, re.compile(p)) for n, p in FEATURES]

KNOWN_MM = {
    'http://www.eclipse.org/emf/2002/Ecore': 'ecore',
    'http://www.eclipse.org/qvt/1.0.0/Operational/examples/simpleuml': 'simpleuml',
    'http://www.eclipse.org/qvt/1.0.0/Operational/examples/rdb': 'rdb',
    'http://www.eclipse.org/m2m/qvt/oml/testqvt': 'testqvt',
    'http://www.eclipse.org/m2m/qvt/oml/generics': 'generics',
    'http://www.eclipse.org/QVT/1.0.0/Operational': 'qvtoperational-ast',
    'http://www.eclipse.org/qvt/1.0.0/Operational/Expressions': 'qvto-expressions-ast',
    'http://www.eclipse.org/emf/2003/XMLType': 'xmltype',
    'http://www.eclipse.org/uml2/2.1.0/UML': 'uml2',
    'http://www.eclipse.org/uml2/3.0.0/UML': 'uml2',
}
BUILTIN_BBOX = {'Strings': 'legacy StringLibrary (o.e.m2m.qvt.oml.ocl)',
                'emf.tools': 'legacy EmfToolsLibrary (o.e.m2m.qvt.oml.ocl)'}
TEST_BBOX_UNITS = {'org.bar.Foo', 'org.bar.Collections', 'TestBlackboxLibrary'}


def qvto_files(folder):
    res = []
    for dp, dn, fn in os.walk(folder):
        dn.sort()
        for f in sorted(fn):
            if f.endswith('.qvto'):
                res.append(os.path.join(dp, f))
    return res


def scan_qvto(files):
    raw = '\n'.join(rd(f) for f in files)
    nocomment = strip_qvto_comments(raw)
    code = strip_strings(nocomment)
    tags = [n for n, p in FEATURES if p.search(code)]
    mms = set()
    for m in re.finditer(r'\bmodeltype\s+(\w+)\s*(?:"[^"]*"|\'[^\']*\'|\w+)?\s*uses\s+([^;]*);', nocomment, re.S):
        uses = m.group(2)
        uris = re.findall(r'["\']([^"\']+)["\']', uses)
        if not uris:
            mms.add('byname:' + uses.strip())
        for u in uris:
            mms.add(KNOWN_MM.get(u, 'custom:' + u))
    imports = re.findall(r'(?m)^\s*import\s+(?:library\s+)?([\w.]+)\s*;', nocomment)
    bb_builtin = sorted({i for i in imports if i in BUILTIN_BBOX})
    bb_test = sorted({i for i in imports if i in TEST_BBOX_UNITS or i.startswith('org.eclipse.m2m.tests')})
    if bb_builtin:
        tags.append('blackbox_builtin_lib')
    if bb_test:
        tags.append('blackbox_java_import')
    return tags, mms, bb_builtin, bb_test, raw


# ----------------------------------------------------------------- rows
COLS = ['test_id', 'driving_test_class', 'qvto_path', 'inputs', 'expected_outputs',
        'expected_kind', 'needs_java_blackbox', 'metamodels', 'feature_tags', 'notes']
rows = []

ALSO_RUN = ('also run by TestQvtWalker(AST walk only), TestQvtExecutor(TransformationExecutor, in-memory compare), '
            'DebugExecutorTest, TestExecutableTransfCopy(.qvtox copy), TestQvtStandaloneExecutor(file: URIs, StandaloneTests.launch)')


def rel(p):
    return os.path.relpath(p, ROOT)


def add_model_row(test_id, driver, e, expected_kind=None, expected_override=None, extra_tags=(), notes=''):
    folder = os.path.join(ROOT, 'tests', e['bundle'], e['data_folder'], 'models', e['name'])
    main = os.path.join(folder, e['name'] + '.qvto')
    files = qvto_files(folder)
    tags, mms, bb_builtin, bb_test, raw = scan_qvto(files) if files else ([], set(), [], [], '')
    tags += list(extra_tags)
    for mm in e['mms']:
        mms.add('file:' + mm)
    pk = {'SimpleumlPackage': 'simpleuml', 'ExpressionsPackage': 'qvto-expressions-ast', 'GenericsPackage': 'generics',
          'TestqvtPackage': 'testqvt', 'TracePackage': 'qvto-trace', 'Bug577992Package': 'custom:http://bug577992'}
    for p in e.get('used_packages', []):
        mms.add(pk.get(p, p))
    expected = expected_override if expected_override is not None else e['expected']
    if expected_kind is None:
        if expected:
            expected_kind = 'model'
        elif re.search(r'\bassert\b', strip_qvto_comments(raw)):
            expected_kind = 'assert'
        else:
            expected_kind = 'none'
    java_dir = os.path.isdir(os.path.join(folder, 'src'))
    test_bbox = [c for c in e.get('bbox_classes', []) if c not in ('StringLibrary', 'EmfToolsLibrary')]
    needs_java = bool(test_bbox) or bool(bb_test) or java_dir
    if len(test_bbox) < len(e.get('bbox_classes', [])) and 'blackbox_builtin_lib' not in tags:
        tags.append('blackbox_builtin_lib')
    if java_dir:
        tags.append('workspace_java_blackbox')
    if e['props']:
        tags.append('config_props_supplied')
    if e.get('session_opts'):
        tags.append('session_options')
    if e['kind'] in ('ReferencedProjectData', 'PluginDependencyProjectData'):
        tags.append('workspace_project_reference' if e['kind'] == 'ReferencedProjectData' else 'pde_plugin_dependency')
    if len(e['inputs']) > 1 or (expected_kind == 'model' and len(expected) > 1):
        tags.append('multi_model')
    if os.path.exists(os.path.join(folder, e['name'] + '.qvtotrace')):
        tags.append('incremental_update_trace')
    if any(m.startswith('file:') or m.startswith('custom:') for m in mms) or \
            any(m in ('simpleuml', 'rdb', 'testqvt', 'generics', 'uml2') for m in mms):
        tags.append('non_ecore_metamodel')
    else:
        tags.append('ecore_only')
    n = []
    if notes:
        n.append(notes)
    if e['props']:
        n.append('cfg=' + ';'.join('%s=%s' % kv for kv in e['props']))
    if e.get('bbox_classes'):
        n.append('bbox=' + ','.join(c.split('.')[-1] for c in e['bbox_classes']))
    if bb_builtin:
        n.append('builtin_bbox=' + ','.join(bb_builtin))
    if e.get('severity'):
        n.append('expected_severity=' + e['severity'])
    if e.get('session_opts'):
        n.append('session=' + ','.join('%s=%s' % kv for kv in e['session_opts']))
    if e.get('referenced'):
        n.append('referenced_project=%s cycle=%s' % (e['referenced'], e['cycle']))
    if not os.path.exists(main):
        n.append('MAIN_QVTO_MISSING')
    missing = [x for x in list(e['inputs']) + list(expected)
               if '=' not in x and not os.path.exists(os.path.join(folder, x))]
    if missing:
        n.append('missing_files=' + ','.join(missing))
    rows.append(OrderedDict(
        test_id=test_id, driving_test_class=driver,
        qvto_path=rel(main), inputs=','.join(e['inputs']), expected_outputs=','.join(expected),
        expected_kind=expected_kind, needs_java_blackbox='y' if needs_java else 'n',
        metamodels=','.join(sorted(mms)) or 'none', feature_tags=','.join(dict.fromkeys(tags)),
        notes=' | '.join(n)))


def uniq_id(base):
    ids = {r['test_id'] for r in rows}
    if base not in ids:
        return base
    i = 2
    while '%s#%d' % (base, i) in ids:
        i += 1
    return '%s#%d' % (base, i)


T = os.path.join(SRC, 'transform')
# 1. TransformTests.createTestData() -> TestQvtInterpreter (+ reuse)
tt = strip_java_comments(rd(os.path.join(T, 'TransformTests.java')))
tt_entries = extract_entries(method_body(tt, 'createTestData'))
jl = strip_java_comments(rd(os.path.join(T, 'javaless/JavalessQvtTest.java')))
jl_ex = set(STR.findall(jl[jl.index('Set<String> JAVALESS_EXCLUDES'):jl.index('Set<String> JAVALESS_PATCH_OUTPUT')]))
tc = strip_java_comments(rd(os.path.join(SRC, 'callapi/TestExecutableTransfCopy.java')))
tc_ban = set(STR.findall(tc[tc.index('BANNED_TASKS'):tc.index('));', tc.index('BANNED_TASKS'))]))
for e in tt_entries:
    javaless = (e['kind'] in ('FileToFileData', 'FilesToFilesData')
                and all(x.endswith('.ecore') for x in e['inputs'] + e['expected'])
                and e['name'] not in jl_ex)
    n = ALSO_RUN + ('; JavalessQvtTest' if javaless else '; not javaless')
    if e['name'] in tc_ban:
        n += '; excluded from TestExecutableTransfCopy'
    tid = 'interpret_' + e['name']
    if e['kind'] in ('ReferencedProjectData', 'PluginDependencyProjectData'):
        tid += '_cycle' if e['cycle'] else '_nocycle'
    add_model_row(uniq_id(tid), 'transform.TestQvtInterpreter', e, notes=n)
interp_count = len(tt_entries)

# 2. TransformManyModelTests
tm = strip_java_comments(rd(os.path.join(T, 'TransformManyModelTests.java')))
for e in extract_entries(tm):
    add_model_row(uniq_id('manymodel_' + e['name']), 'transform.TransformManyModelTests', e)

# 3. TestFailedTransformation -> expected log (qvtTraceLog.stack)
tf = strip_java_comments(rd(os.path.join(T, 'TestFailedTransformation.java')))
for e in extract_entries(tf):
    add_model_row(uniq_id('failed_' + e['name']), 'transform.TestFailedTransformation', e,
                  expected_kind='log', expected_override=['qvtTraceLog.stack'],
                  extra_tags=['runtime_failure_expected'],
                  notes='log written via WriterLog must equal file (incl. QVT stack trace)')

# 4. TestIncorrectTransformation -> build must fail
ti = strip_java_comments(rd(os.path.join(T, 'TestIncorrectTransformation.java')))
for e in extract_entries(ti):
    add_model_row(uniq_id('incorrect_%s_%s' % (e['name'], 'cycle' if e['cycle'] else 'nocycle')),
                  'transform.TestIncorrectTransformation', e, expected_kind='error', expected_override=[],
                  extra_tags=['compile_error_expected'], notes='passes iff workspace build reports an error marker')

# 5. Stack trace style tests (AbstractStackTraceTest subclasses)
st_cases = [
    ('stacktrace', 'TestStackTrace', 'testMappingCallInProperty', 'mappingCallInProperty', ['mappingCallInProperty.stack'], 'stack', {}),
    ('stacktrace', 'TestStackTrace', 'testLogExpUsage', 'testLogExpUsage', ['testLogExpUsage.log.txt'], 'log', {}),
    ('stacktrace', 'TestStackTrace', 'testAssertionFailed', 'assertionFailed', ['assertionFailed.stack', 'assertionFailed.log.txt'], 'stack', {}),
    ('stacktrace', 'TestStackTrace', 'testStackOverFlow', 'stackOverFlow', [], 'stack', {}),
    ('stacktrace', 'TestStackTrace', 'testCreateInstaceFailure', 'createInstaceFailure', ['createInstaceFailure.stack'], 'stack', {}),
    ('stacktrace', 'TestStackTrace', 'testUknownSourceStackTrace', 'createInstaceFailure', ['testUknownSourceStackTrace.stack'], 'stack', {}),
    ('invalidConfigProp', 'TestInvalidConfigProperty', 'testInvalidConfigProp', 'invalidConfigProp', ['invalidIntConfigProp.stack'], 'stack', {'intProp': 'xxx'}),
    ('invalidConfigProp', 'TestInvalidConfigProperty', 'testInvalidRealConfigProp', 'invalidConfigProp', ['invalidRealConfigProp.stack'], 'stack', {'realProp': 'xxx'}),
    ('invalidConfigProp', 'TestInvalidConfigProperty', 'testUndefinedConfigProp', 'invalidConfigProp', [], 'none', {}),
    ('blackboxlib_context', 'TestBlackboxLibContext', 'testInvalidConfigProp', 'executionTerminated', ['executionTerminated.stack', 'executionTerminated.log.txt'], 'stack', {'strTest': 'test', 'strTest2': 'test2'}),
]
for folder, cls, meth, tcname, exp, kind, props in st_cases:
    e = make_entry('FileToFileData', [('str', folder)], '', [])
    e['props'] = list(props.items()) + [('testcase', tcname)]
    note = 'config prop testcase=%s selects scenario; .stack = QvtRuntimeException.printQvtStackTrace, .log.txt = log output' % tcname
    if meth == 'testStackOverFlow':
        note += '; asserts QvtStackOverFlowError + frame names programmatically'
    if meth == 'testUknownSourceStackTrace':
        note += '; compiled without line numbers'
    if cls == 'TestBlackboxLibContext':
        e['bbox_classes'] = []
    add_model_row(uniq_id('%s_%s' % (folder, meth)), 'transform.' + cls, e, expected_kind=kind,
                  expected_override=exp, extra_tags=['runtime_failure_expected'] if kind == 'stack' else [], notes=note)

# 6. Trace file tests
for name, n in (('multipletracerecords', 5), ('mappingWithNoResultTrace_266854', 1)):
    e = make_entry('FileToFileData', [('str', name)], '', [])
    add_model_row(uniq_id('trace_' + name), 'traces.TestTraceFile', e, expected_kind='trace',
                  expected_override=['trace_records=%d' % n], extra_tags=['trace_persistence'],
                  notes='asserts number of trace records in saved .qvtotrace')

# 7. Parser tests (TestQvtParser + subclasses)
tp = strip_java_comments(rd(os.path.join(SRC, 'TestQvtParser.java')))
parser_entries = []
for m in re.finditer(r'(TestData\.createSourceChecked|new\s+TestData)\(\s*"([^"]+)"\s*,\s*(-?\d+)\s*(?:,\s*(-?\d+))?\s*\)((?:\s*\.includeMetamodel\(\s*"[^"]*"\s*\))*)',
                     method_body(tp, 'data')):
    parser_entries.append(('parser', 'TestQvtParser', m.group(2), int(m.group(3)),
                           int(m.group(4)) if m.group(4) else -1, m.group(1).startswith('TestData.create'),
                           re.findall(r'"([^"]*)"', m.group(5))))
parser_entries += [
    ('parser', 'Bug577992', 'bug577992', 0, 0, True, ['(mapping file -> bug577992.ecore)']),
    ('parser', 'Bug578265', 'bug578265', 0, 0, True, []),
    ('parser', 'Bug579914', 'bug579914', 0, 0, True, []),
    ('parser', 'QvtLibraryASTTest', 'blackboxlibASTmodel', 0, 0, False, []),
]
for _, cls, d, errs, warns, annotated, mms in parser_entries:
    folder = os.path.join(ROOT, BUNDLE, 'parserTestData/sources', d)
    files = qvto_files(folder)
    tags, mmset, bb_builtin, bb_test, raw = scan_qvto(files) if files else ([], set(), [], [], '')
    n_err_ann = len(re.findall(r'/\*<error', raw))
    n_warn_ann = len(re.findall(r'/\*<warn', raw))
    for mm in mms:
        mmset.add('file:' + mm)
    java_dir = os.path.isdir(os.path.join(folder, 'src'))
    tags.append('compile_only')
    if annotated:
        tags.append('source_annotated_problems')
    kind = 'error' if (errs > 0 or warns > 0) else 'compile_ok'
    notes = 'errors=%d warnings=%s annotated=%s (annotations in sources: %d error, %d warn)' % (
        errs, 'unchecked' if warns < 0 else warns, 'y' if annotated else 'n', n_err_ann, n_warn_ann)
    if bb_builtin:
        notes += ' | builtin_bbox=' + ','.join(bb_builtin)
    rows.append(OrderedDict(
        test_id=uniq_id('parse_' + d), driving_test_class=cls,
        qvto_path=rel(os.path.join(folder, d + '.qvto')), inputs='', expected_outputs='errors=%d;warnings=%s' % (errs, 'any' if warns < 0 else warns),
        expected_kind=kind, needs_java_blackbox='y' if (java_dir or bb_test or 'blackbox' in d) else 'n',
        metamodels=','.join(sorted(mmset)) or 'none', feature_tags=','.join(dict.fromkeys(tags)), notes=notes))
parser_count = len(parser_entries)

# 8. API-level tests (best effort, manually curated from driver sources)
API = [
    ('api_exec1', 'transform.api.ExecTransformationTestCase', 'apiTestData/exec1/exec1.qvto', 'in1.ecore,in2.ecore', 'expected.simpleuml,expected.rdb', 'model', 'n', 'also ExecDeployedTransformationTestCase(+UseFileName) via plugin.xml qvtTransformation id'),
    ('api_exec2', 'transform.api.ExecTransformationTestCase', 'apiTestData/exec2/exec2.qvto', 'in1.ecore,in2.ecore', 'expected.simpleuml,expected.rdb', 'model', 'n', 'also ExecDeployedTransformationTestCase'),
    ('api_exec3', 'transform.api.ExecTransformationTestCase', 'apiTestData/exec3/exec3.qvto', '', '', 'none', 'n', 'deployed via plugin.xml id apiTestData.exec3.exec3'),
    ('api_exec3_withImport', 'transform.api.ExecDeployedTransformationTestCase', 'apiTestData/exec3_withImport/exec3_withImport.qvto', '', '', 'none', 'n', 'deployed transformation with import'),
    ('api_exec_erroneous', 'transform.api.WrappedExecTransformationTestCase', 'apiTestData/exec_erroneous/exec_erroneous.qvto', '', '', 'error', 'n', 'runtime failure surfaced through wrapped API'),
    ('api_imports', 'transform.api.ImportedTransfTestCase', 'apiTestData/imports/imports.qvto', '', 'imports.qvto,q1.qvto,q2.qvto,nested2/q4.qvto,nested/q3.qvto', 'none', 'n', 'checks resolved import set, not models'),
    ('api_twoInputs', 'transform.api.InputExtensionsTestCase', 'apiTestData/twoInputs/twoInputs.qvto', 'in.simpleuml,in.rdb', 'out.ecore', 'model', 'n', 'input file-extension handling'),
    ('api_inoutModels', 'transform.api.InoutValidationTestCase', '', 'empty.ecore,nonEmpty.ecore,invalid.ecore', '', 'none', 'n', 'validation of inout model URIs (workspace vs platform:/plugin)'),
    ('callapi_Ecore2Ecore', 'callapi.InvocationTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/callapi/Ecore2Ecore.qvto', '(in-memory)', '(programmatic)', 'api', 'n', '16 @Test: success, logging, assert-failed, stack trace, interruption, config props incl. Dict/Set parsing, null/less params'),
    ('callapi_CompilationErrors', 'callapi.InvocationTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/callapi/CompilationErrors.qvto', '', '', 'error', 'n', 'ExecutionDiagnostic for compile errors'),
    ('callapi_NonExecutable', 'callapi.InvocationTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/callapi/NonExecutable.qvto', '', '', 'error', 'n', 'library / no main'),
    ('callapi_Ecore2EcoreExt', 'callapi.InvocationExtTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/callapi/Ecore2EcoreExt.qvto', '(in-memory)', '(programmatic)', 'api', 'n', 'in, inout, out1, out2 extents'),
    ('callapi_InplaceEcore', 'callapi.InvocationExtTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/callapi/InplaceEcore.qvto', '(in-memory)', '(programmatic)', 'api', 'n', 'inout in-place; also Bug431055 with Bug431055.ecore'),
    ('callapi_ChildInTreeInput', 'callapi.ChildInTreeInputInvocationTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/callapi/ChildInTreeInput.qvto', '(in-memory)', '(programmatic)', 'api', 'n', 'input extent holds non-root object'),
    ('callapi_nonregisteredDynamic', 'callapi.DynamicModelInvocationTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/dynamicmodel/nonregisteredDynamic.qvto', 'nonregisteredDynClass.xmi', '(programmatic)', 'api', 'n', 'dynamic EMF, metamodel nonregistered.ecore'),
    ('callapi_registeredDynamic', 'callapi.DynamicRegisteredModelInvocationTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/dynamicmodel/registeredDynamic.qvto', 'registeredDynClass.xmi', '(programmatic)', 'api', 'n', 'dynamic package registered via plugin.xml dynamic_package'),
    ('callapi_bug416584_visitor', 'callapi.TransformationExecutorWithVisitorDecoratorTest', 'tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/models/bug416584/bug416584.qvto', 'in.ecore', '', 'none', 'n', 'evaluation visitor decorator hook'),
    ('callapi_bug463572_incremental', 'callapi.IncrementalUpdateExecutorTest', 'tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/models/bug463572/bug463572.qvto', 'in.ecore,bug463572.qvtotrace', 'expected.ecore', 'model', 'n', 'incremental update from persisted trace'),
    ('callapi_bug326871_standalone', 'callapi.BlackboxModuleStandaloneSetupTest', 'tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/models/bug326871_standalone/bug326871_standalone.qvto', '', '', 'assert', 'y', 'StandaloneBBoxModuleRegistered registered via TransformationExecutor.BlackboxRegistry'),
    ('callapi_bug326871a_standalone', 'callapi.BlackboxModuleStandaloneSetupTest', 'tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/models/bug326871a_standalone/bug326871a_standalone.qvto', '', '', 'assert', 'y', 'StandaloneBBoxModuleRegistered'),
    ('intermprop_cleanup', 'IntermediatePropertyCleanupTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/intermprop_cleanup/CheckMemoryLeak.qvto', '(in-memory)', '(programmatic)', 'api', 'n', 'intermediate property values removed after execution'),
    ('perf_WarmUp', 'perf.ExecutionTimeGuardTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/perf/WarmUp.qvto', '', '', 'perf', 'n', '<8000 ms'),
    ('perf_traceLookup_287589', 'perf.ExecutionTimeGuardTest', 'tests/org.eclipse.m2m.tests.qvt.oml/deployed/perf/traceLookup_287589.qvto', '', '', 'perf', 'n', '<5000 ms'),
    ('ant_simple', 'ant.TestQvtAntScript', 'tests/org.eclipse.m2m.tests.qvt.oml/antTestData/simple/simple.qvto', 'in.ecore', 'expected.ecore,expected.uml', 'model', 'n', 'driven by composite.xml Ant script (platform:/ URIs)'),
    ('ant_twomodels', 'ant.TestQvtAntScript', 'tests/org.eclipse.m2m.tests.qvt.oml/antTestData/twomodels/ecore2uml.qvto', 'in.ecore', 'expected.rdb', 'model', 'n', 'chained ecore2uml + uml2rdb via Ant'),
    ('ant_twomodels_new', 'ant.TestQvtAntScript', 'tests/org.eclipse.m2m.tests.qvt.oml/antTestData/twomodels_new/ecore2uml.qvto', 'in.ecore', 'expected.rdb', 'model', 'n', 'chained via Ant (new syntax)'),
    ('builder_Simpleuml_To_Rdb', 'TestQvtBuilder', 'tests/org.eclipse.m2m.tests.qvt.oml/builderTestData/Simpleuml_To_Rdb.qvto', '', '', 'compile_ok', 'n', 'workspace builder / markers'),
    ('externlib_FooLib', 'TestExternHelperCall', 'tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/externlib/FooLib.qvto', '(in-memory)', '(programmatic)', 'api', 'n', '10 @Test: calling QVTo helpers/queries from Java'),
    ('sourcecontainer_RootTransformation', 'callapi.SourceContainerTests', 'tests/org.eclipse.m2m.tests.qvto.transformationProject/transforms/root/RootTransformation.qvto', '', '', 'compile_ok', 'n', 'source-container / deployed-transformation resolution across plugin projects'),
]
for tid, cls, path, ins, exp, kind, java, note in API:
    if path and not path.startswith('tests/'):
        path = BUNDLE + '/' + path
    full = os.path.join(ROOT, path) if path else ''
    tags, mms = [], set()
    if full and os.path.exists(full):
        tags, mms, _, bb_test, _ = scan_qvto(qvto_files(os.path.dirname(full)))
    tags.append('api_level')
    rows.append(OrderedDict(test_id=tid, driving_test_class=cls, qvto_path=path, inputs=ins, expected_outputs=exp,
                            expected_kind=kind, needs_java_blackbox=java, metamodels=','.join(sorted(mms)) or 'none',
                            feature_tags=','.join(dict.fromkeys(tags)), notes=note))

with open(OUT, 'w', encoding='utf-8') as f:
    f.write('\t'.join(COLS) + '\n')
    for r in rows:
        f.write('\t'.join(str(r[c]).replace('\t', ' ').replace('\n', ' ') for c in COLS) + '\n')

# ----------------------------------------------------------------- summary
print('rows', len(rows))
print('TransformTests entries', interp_count, 'parser entries', parser_count)
print('by driver', Counter(r['driving_test_class'] for r in rows).most_common())
print('by kind', Counter(r['expected_kind'] for r in rows).most_common())
model_rows = [r for r in rows if r['driving_test_class'].startswith(('transform.', 'traces.'))]
print('model-suite rows', len(model_rows), 'java_bbox', sum(r['needs_java_blackbox'] == 'y' for r in model_rows))
print('model-suite kinds', Counter(r['expected_kind'] for r in model_rows))
print('model-suite ecore_only', sum('ecore_only' in r['feature_tags'] for r in model_rows))
print('missing main', [r['test_id'] for r in rows if 'MISSING' in r['notes']])
print('missing files', [(r['test_id'], r['notes'].split('missing_files=')[1].split(' |')[0]) for r in rows if 'missing_files=' in r['notes']])
fc = Counter()
fex = {}
for r in model_rows:
    for t in r['feature_tags'].split(','):
        if t:
            fc[t] += 1
            fex.setdefault(t, []).append(r['test_id'])
pc = Counter()
for r in rows:
    if r['driving_test_class'] not in [x for x in ()] and r['test_id'].startswith('parse_'):
        for t in r['feature_tags'].split(','):
            if t:
                pc[t] += 1
print('\nFEATURES (execution suite rows) / parser rows / examples')
for t, c in fc.most_common():
    ex = [x for x in fex[t] if not x.startswith(('stacktrace_', 'invalidConfigProp_'))][:4]
    print('%-30s %4d %4d  %s' % (t, c, pc.get(t, 0), ', '.join(ex)))
