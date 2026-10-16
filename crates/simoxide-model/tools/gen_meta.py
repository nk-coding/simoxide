#!/usr/bin/env python3
"""Generates src/meta/generated.rs (metamodel tables) and extracts the bundled pathmap models.

Reads the .ecore files and default models straight from the SimuLizar 5.2.2 product jars, so the
tables are exactly the metamodels the reference runs with.

Usage: tools/gen_meta.py [PLUGINS_DIR]   (run from crates/simoxide-model)
"""
import os, re, sys, zipfile, glob
import xml.etree.ElementTree as ET

PLUGINS = sys.argv[1] if len(sys.argv) > 1 else \
    '/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2/plugins'
XSI = '{http://www.w3.org/2001/XMLSchema-instance}type'

# (bundle symbolic name, path of the .ecore inside the jar)
ECORES = [
    ('de.uka.ipd.sdq.identifier', 'model/identifier.ecore'),
    ('de.uka.ipd.sdq.units', 'model/Units.ecore'),
    ('de.uka.ipd.sdq.probfunction', 'model/ProbabilityFunction.ecore'),
    ('de.uka.ipd.sdq.stoex', 'model/stoex.ecore'),
    ('org.palladiosimulator.pcm', 'model/pcm.ecore'),
    ('org.palladiosimulator.metricspec', 'model/metricspec.ecore'),
    ('org.palladiosimulator.edp2', 'model/EDP2.ecore'),
    ('org.palladiosimulator.monitorrepository', 'model/monitorrepository.ecore'),
    ('org.palladiosimulator.monitorrepository.map', 'model/map.ecore'),
    ('org.palladiosimulator.pcm.edp2.measuringpoint', 'model/pcmmeasuringpoint.ecore'),
    ('org.palladiosimulator.simulizar.edp2.measuringpoint', 'model/simulizarmeasuringpoint.ecore'),
]
# bundled default models: (bundle, path in jar, uri prefix it is served under, file name)
MODELS = [
    ('org.palladiosimulator.pcm.resources', 'defaultModels/Palladio.resourcetype'),
    ('org.palladiosimulator.pcm.resources', 'defaultModels/PrimitiveTypes.repository'),
    ('org.palladiosimulator.pcm.resources', 'defaultModels/FailureTypes.repository'),
    ('org.palladiosimulator.pcm.resources', 'defaultModels/Glassfish.repository'),
    ('org.palladiosimulator.pcm.resources', 'defaultModels/default_event_middleware.repository'),
    ('org.palladiosimulator.metricspec.resources', 'models/commonMetrics.metricspec'),
]

def jar_for(bsn):
    c = sorted(glob.glob(os.path.join(PLUGINS, bsn + '_*.jar')))
    c = [x for x in c if re.match(re.escape(bsn) + r'_\d', os.path.basename(x))]
    if not c:
        sys.exit('no jar for ' + bsn)
    return c[-1]

BUILTIN = {  # Ecore data types -> kind
    'EString': 'Str', 'EInt': 'Int', 'EIntegerObject': 'Int', 'ELong': 'Long', 'ELongObject': 'Long',
    'EDouble': 'Double', 'EDoubleObject': 'Double', 'EFloat': 'Double', 'EBoolean': 'Bool',
    'EBooleanObject': 'Bool', 'EShort': 'Int', 'EByte': 'Int', 'EChar': 'Other', 'EBigDecimal': 'Other',
    'EBigInteger': 'Other', 'EDate': 'Other', 'EJavaObject': 'Other', 'EJavaClass': 'Other',
    'EFeatureMapEntry': 'Other', 'EEList': 'Other', 'EMap': 'Other', 'EResource': 'Other',
    'EDiagnosticChain': 'Other', 'ETreeIterator': 'Other', 'EByteArray': 'Other',
}

files = {}  # basename -> root
nsfile = {}  # nsURI -> basename (top package)
for bsn, path in ECORES:
    z = zipfile.ZipFile(jar_for(bsn))
    root = ET.fromstring(z.read(path))
    files[os.path.basename(path)] = root
    nsfile[root.get('nsURI')] = os.path.basename(path)

packages = []  # dicts
classes = []
enums = []
datatypes = {}
by_path = {}  # (file, 'a/b/Name') -> ('class'|'enum'|'dt', index)

datatypes_list = []  # data types defined in the metamodels (not Ecore's)

def walk(fname, pkg, qual, pathprefix, parent=None, loc=None):
    name = pkg.get('name')
    q = qual + ('.' if qual else '') + name
    pi = len(packages)
    packages.append(dict(name=q, short=name, ns=pkg.get('nsURI'), prefix=pkg.get('nsPrefix'), classes=[],
                         order=[], sup=parent, loc=loc))
    for c in pkg.findall('eClassifiers'):
        t = c.get(XSI)
        p = pathprefix + c.get('name')
        if t == 'ecore:EClass':
            ci = len(classes)
            classes.append(dict(name=c.get('name'), pkg=pi, abstract=c.get('abstract') == 'true' or c.get('interface') == 'true',
                                interface=c.get('interface') == 'true', abstract_attr=c.get('abstract') == 'true',
                                icn=c.get('instanceClassName'), el=c, file=fname, supers=[], feats=[]))
            packages[pi]['classes'].append(ci)
            packages[pi]['order'].append(('Class', ci))
            by_path[(fname, p)] = ('class', ci)
        elif t == 'ecore:EEnum':
            ei = len(enums)
            lits = []
            nextv = 0
            for l in c.findall('eLiterals'):
                v = int(l.get('value')) if l.get('value') is not None else nextv
                nextv = v + 1
                lits.append((l.get('name'), v, l.get('literal') if l.get('literal') is not None else l.get('name')))
            enums.append(dict(name=c.get('name'), pkg=pi, lits=lits))
            by_path[(fname, p)] = ('enum', ei)
            packages[pi]['order'].append(('Enum', ei))
        else:
            di = len(datatypes_list)
            datatypes_list.append(dict(name=c.get('name'), pkg=pi, icn=c.get('instanceClassName'),
                                       serializable=c.get('serializable', 'true') == 'true'))
            by_path[(fname, p)] = ('dt', c.get('name'), c.get('instanceClassName'), di)
            packages[pi]['order'].append(('DataType', di))
    for sp in pkg.findall('eSubpackages'):
        walk(fname, sp, q, pathprefix + sp.get('name') + '/', pi)

locations = {}  # file basename -> platform:/plugin URI of the .ecore
for bsn, path in ECORES:
    locations[os.path.basename(path)] = f'platform:/plugin/{bsn}/{path}'
for fname, root in files.items():
    walk(fname, root, '', '', None, locations[fname])

def resolve(ref, fname):
    """ref like '#//a/B', '../../x/model/pcm.ecore#//core/Entity', 'http://...#//X',
    'ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EString'"""
    ref = ref.split()[-1]
    loc, frag = ref.split('#', 1)
    frag = frag[2:]
    if loc == '':
        f = fname
    elif 'eclipse.org/emf/2002/Ecore' in loc or loc.endswith('Ecore.ecore'):
        return ('ecore', frag)
    elif loc in nsfile:
        f = nsfile[loc]
    else:
        f = os.path.basename(loc)
    if (f, frag) not in by_path:
        raise KeyError((ref, fname))
    return by_path[(f, frag)]

def type_ref(et, fname):
    """The classifier an eType/eClassifier reference designates, for the extension tables."""
    if et is None:
        return 'TypeRef::Unknown'
    r = resolve(et, fname)
    if r[0] == 'ecore':
        return f'TypeRef::Ecore({rs_str(r[1])})'
    if r[0] == 'class':
        return f'TypeRef::Class(ClassId({r[1]}))'
    if r[0] == 'enum':
        return f'TypeRef::Enum(EnumId({r[1]}))'
    return f'TypeRef::DataType({r[3]})'

def generic_type(el):
    et = el.get('eType')
    if et is None:
        g = el.find('eGenericType')
        et = g.get('eClassifier') if g is not None else None
    return et

def rs_str(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'

features = []
for ci, c in enumerate(classes):
    el = c['el']
    for s in (el.get('eSuperTypes') or '').split():
        r = resolve(s, c['file'])
        if r[0] == 'ecore':
            continue  # EObject
        c['supers'].append(r[1])
for ci, c in enumerate(classes):
    for f in c['el'].findall('eStructuralFeatures'):
        t = f.get(XSI)
        et = f.get('eType')
        if et is None:
            g = f.find('eGenericType')
            et = g.get('eClassifier') if g is not None else None
        d = dict(name=f.get('name'), owner=ci, attr=(t == 'ecore:EAttribute'),
                 lower=int(f.get('lowerBound', '0')), upper=int(f.get('upperBound', '1')),
                 containment=f.get('containment') == 'true', transient=f.get('transient') == 'true',
                 volatile=f.get('volatile') == 'true', derived=f.get('derived') == 'true',
                 changeable=f.get('changeable', 'true') == 'true', unsettable=f.get('unsettable') == 'true',
                 resolve_proxies=f.get('resolveProxies', 'true') == 'true', is_id=f.get('iD') == 'true',
                 default=f.get('defaultValueLiteral'), opp=f.get('eOpposite'), file=c['file'],
                 ordered=f.get('ordered', 'true') == 'true', unique=f.get('unique', 'true') == 'true',
                 tref=type_ref(et, c['file']))
        if et is None:
            d['dt'] = ('Other', None)
        else:
            r = resolve(et, c['file'])
            if d['attr']:
                if r[0] == 'ecore':
                    d['dt'] = (BUILTIN.get(r[1], 'Other'), None)
                elif r[0] == 'enum':
                    d['dt'] = ('Enum', r[1])
                else:
                    icn = r[2] or ''
                    kind = {'java.lang.String': 'Str', 'int': 'Int', 'double': 'Double', 'boolean': 'Bool',
                            'long': 'Long'}.get(icn, 'Other')
                    d['dt'] = (kind, None)
            else:
                d['target'] = r[1] if r[0] == 'class' else None
        fi = len(features)
        features.append(d)
        c['feats'].append(fi)
# opposites: '#//pkg/Class/feature'
for fi, f in enumerate(features):
    if f['opp']:
        loc, frag = f['opp'].split('#', 1)
        parts = frag[2:].split('/')
        cls = by_path[(f['file'] if loc == '' else os.path.basename(loc), '/'.join(parts[:-1]))][1]
        f['opp_idx'] = next(x for x in classes[cls]['feats'] if features[x]['name'] == parts[-1])
    else:
        f['opp_idx'] = None

memo_sup = {}
def all_supers(ci):
    if ci in memo_sup:
        return memo_sup[ci]
    res = []
    for s in classes[ci]['supers']:
        for x in all_supers(s) + [s]:
            if x not in res:
                res.append(x)
    memo_sup[ci] = res
    return res
memo_feat = {}
def all_feats(ci):
    if ci in memo_feat:
        return memo_feat[ci]
    res = []
    for s in classes[ci]['supers']:
        for x in all_feats(s):
            if x not in res:
                res.append(x)
    res += classes[ci]['feats']
    memo_feat[ci] = res
    return res

# constant names
cls_count = {}
for c in classes:
    cls_count[c['name']] = cls_count.get(c['name'], 0) + 1
def cls_const(ci):
    c = classes[ci]
    if cls_count[c['name']] > 1:
        return packages[c['pkg']]['short'] + '_' + c['name']
    return c['name']

out = []
w = out.append
w('// @generated by tools/gen_meta.py from the SimuLizar 5.2.2 product jars. Do not edit.')
w('#![allow(non_upper_case_globals, clippy::all)]')
w('use super::*;')
w('')
w('pub static PACKAGES: &[PackageDef] = &[')
for p in packages:
    w(f'    PackageDef {{ name: {rs_str(p["name"])}, ns_uri: {rs_str(p["ns"])}, prefix: {rs_str(p["prefix"])} }},')
w('];')
w('pub static ENUMS: &[EnumDef] = &[')
for e in enums:
    lits = ', '.join(f'EnumLiteral {{ name: {rs_str(n)}, value: {v}, literal: {rs_str(l)} }}' for n, v, l in e['lits'])
    w(f'    EnumDef {{ name: {rs_str(e["name"])}, package: PackageId({e["pkg"]}), literals: &[{lits}] }},')
w('];')
w('pub static CLASSES: &[ClassDef] = &[')
for ci, c in enumerate(classes):
    sup = ', '.join(f'ClassId({x})' for x in c['supers'])
    asup = ', '.join(f'ClassId({x})' for x in all_supers(ci))
    af = ', '.join(f'FeatureId({x})' for x in all_feats(ci))
    idattr = next((x for x in all_feats(ci) if features[x]['attr'] and features[x]['is_id']), None)
    ida = f'Some(FeatureId({idattr}))' if idattr is not None else 'None'
    w(f'    ClassDef {{ name: {rs_str(c["name"])}, package: PackageId({c["pkg"]}), is_abstract: {str(c["abstract"]).lower()}, '
      f'supers: &[{sup}], all_supers: &[{asup}], all_features: &[{af}], id_attribute: {ida} }},')
w('];')
w('pub static FEATURES: &[FeatureDef] = &[')
for fi, f in enumerate(features):
    if f['attr']:
        k, e = f['dt']
        dt = f'DataKind::Enum(EnumId({e}))' if k == 'Enum' else f'DataKind::{k}'
        kind = f'FeatureKind::Attribute {{ data: {dt} }}'
    else:
        tgt = f'ClassId({f["target"]})' if f['target'] is not None else 'ClassId(u32::MAX)'
        opp = f'Some(FeatureId({f["opp_idx"]}))' if f['opp_idx'] is not None else 'None'
        container = (f['opp_idx'] is not None and features[f['opp_idx']]['containment'])
        kind = (f'FeatureKind::Reference {{ target: {tgt}, containment: {str(f["containment"]).lower()}, '
                f'container: {str(container).lower()}, opposite: {opp}, resolve_proxies: {str(f["resolve_proxies"]).lower()} }}')
    dflt = f'Some({rs_str(f["default"])})' if f['default'] is not None else 'None'
    w(f'    FeatureDef {{ name: {rs_str(f["name"])}, owner: ClassId({f["owner"]}), kind: {kind}, many: {str(f["upper"] != 1).lower()}, '
      f'lower: {f["lower"]}, transient: {str(f["transient"]).lower()}, derived: {str(f["derived"]).lower()}, '
      f'volatile: {str(f["volatile"]).lower()}, is_id: {str(f["is_id"]).lower()}, default: {dflt} }},')
w('];')
w('')
w('/// Class constants (`package_Name` where the class name is ambiguous).')
w('pub mod class {')
w('    use super::ClassId;')
for ci, c in enumerate(classes):
    w(f'    pub const {cls_const(ci)}: ClassId = ClassId({ci});')
w('}')
w('')
w('/// Feature constants, `<Class>_<feature>` of the declaring class.')
w('pub mod feat {')
w('    use super::FeatureId;')
for fi, f in enumerate(features):
    w(f'    pub const {cls_const(f["owner"])}_{f["name"]}: FeatureId = FeatureId({fi});')
w('}')
w('')
w('/// Enum constants.')
w('pub mod enums {')
w('    use super::EnumId;')
ecount = {}
for e in enums:
    ecount[e['name']] = ecount.get(e['name'], 0) + 1
for ei, e in enumerate(enums):
    n = e['name'] if ecount[e['name']] == 1 else packages[e['pkg']]['short'] + '_' + e['name']
    w(f'    pub const {n}: EnumId = EnumId({ei});')
w('}')
open('src/meta/generated.rs', 'w').write('\n'.join(out) + '\n')

# Extension tables: the EMF facts the loader does not need but a full metamodel registry does
# (multiplicities, ordered/unique/changeable/unsettable, interfaces, data types, classifier
# order, operations, the location of each top-level package), for hosts such as a QVTo engine.
def classifier_ref(k, i):
    return {'Class': f'ClassifierRef::Class(ClassId({i}))', 'Enum': f'ClassifierRef::Enum(EnumId({i}))',
            'DataType': f'ClassifierRef::DataType({i})'}[k]

ext = []
w = ext.append
w('// @generated by tools/gen_meta.py from the SimuLizar 5.2.2 product jars. Do not edit.')
w('#![allow(clippy::all)]')
w('use super::*;')
w('')
w('pub static PACKAGE_EXT: &[PackageExt] = &[')
for p in packages:
    sup = f'Some(PackageId({p["sup"]}))' if p['sup'] is not None else 'None'
    loc = f'Some({rs_str(p["loc"])})' if p['sup'] is None else 'None'
    order = ', '.join(classifier_ref(k, i) for k, i in p['order'])
    w(f'    PackageExt {{ super_package: {sup}, location: {loc}, classifiers: &[{order}] }},')
w('];')
w('pub static CLASS_EXT: &[ClassExt] = &[')
for c in classes:
    icn = f'Some({rs_str(c["icn"])})' if c['icn'] else 'None'
    # eSuperTypes as EMF reports them: explicit Ecore supertypes and generic supertypes included
    # (the main tables skip both)
    sups = [type_ref(x, c['file']) for x in (c['el'].get('eSuperTypes') or '').split()]
    sups += [type_ref(g.get('eClassifier'), c['file']) for g in c['el'].findall('eGenericSuperTypes')]
    w(f'    ClassExt {{ is_abstract: {str(c["abstract_attr"]).lower()}, is_interface: {str(c["interface"]).lower()}, '
      f'instance_class_name: {icn}, supers: &[{", ".join(sups)}] }},')
w('];')
w('pub static FEATURE_EXT: &[FeatureExt] = &[')
for f in features:
    w(f'    FeatureExt {{ upper: {f["upper"]}, ordered: {str(f["ordered"]).lower()}, unique: {str(f["unique"]).lower()}, '
      f'changeable: {str(f["changeable"]).lower()}, unsettable: {str(f["unsettable"]).lower()}, ty: {f["tref"]} }},')
w('];')
w('pub static DATA_TYPES: &[DataTypeDef] = &[')
for d in datatypes_list:
    icn = f'Some({rs_str(d["icn"])})' if d['icn'] else 'None'
    w(f'    DataTypeDef {{ name: {rs_str(d["name"])}, package: PackageId({d["pkg"]}), instance_class_name: {icn}, '
      f'serializable: {str(d["serializable"]).lower()} }},')
w('];')

def typed_def(el, fname):
    up = int(el.get('upperBound', '1'))
    return (f'TypedDef {{ ty: {type_ref(generic_type(el), fname)}, upper: {up}, '
            f'ordered: {str(el.get("ordered", "true") == "true").lower()}, unique: {str(el.get("unique", "true") == "true").lower()} }}')

w('pub static OPERATIONS: &[OperationDef] = &[')
for ci, c in enumerate(classes):
    for op in c['el'].findall('eOperations'):
        res = f'Some({typed_def(op, c["file"])})' if generic_type(op) is not None else 'None'
        params = ', '.join(f'ParamDef {{ name: {rs_str(pp.get("name"))}, ty: {typed_def(pp, c["file"])} }}'
                           for pp in op.findall('eParameters'))
        w(f'    OperationDef {{ owner: ClassId({ci}), name: {rs_str(op.get("name"))}, result: {res}, params: &[{params}] }},')
w('];')
open('src/meta/generated_ext.rs', 'w').write('\n'.join(ext) + '\n')

for bsn, path in MODELS:
    z = zipfile.ZipFile(jar_for(bsn))
    open(os.path.join('models', os.path.basename(path)), 'wb').write(z.read(path))
print(f'{len(packages)} packages, {len(classes)} classes, {len(features)} features, {len(enums)} enums')
