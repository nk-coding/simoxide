#!/usr/bin/env python3
"""Writes corpus/INDEX.md from the model files, run.json, FEATURES.txt / external-models.txt and the
output of `refsim batch <corpus> --check` (timings, t_end, uniforms, measurement counts).
Usage: tools/mkindex.py <batch-log> [corpusDir]"""
import json, os, re, sys

log, corpus = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(__file__), '../../corpus'))
corpus = os.path.normpath(corpus)
ref = os.path.normpath(os.path.join(os.path.dirname(__file__), '..'))

runs = {}
for line in open(log):
    m = re.match(r'^(\S+)\s+(\S+)\s+(\d+) ms\s+t_end=(\S+)\s+uniforms=(\d+)\s+meas=(\d+)\s+trace=(\d+)\s+(.*)$', line)
    if m:
        runs[m.group(1)] = dict(ms=int(m.group(3)), t_end=m.group(4), uniforms=int(m.group(5)), meas=int(m.group(6)),
                                status=m.group(8).strip())

sources = {}
for line in open(os.path.join(ref, 'external-models.txt')):
    if line.startswith('#') or not line.strip():
        continue
    name, src, *_ = line.rstrip('\n').split('|')
    src = src.replace('repos/Palladio-Analyzer-Slingshot-E2E-Tests/bundles/org.palladiosimulator.analyzer.slingshot.e2e.helpers/src/main/resources/', 'Slingshot-E2E:')
    src = src.replace('repos/Palladio-Analyzer-SimuLizar/tests/org.palladiosimulator.simulizar.tests/testmodels/', 'SimuLizar-tests:')
    src = src.replace('repos/Palladio-Analyzer-SimuLizar/bundles/ExampleModels/org.palladiosimulator.simulizar.examples.', 'SimuLizar-examples:')
    src = src.replace('repos/Palladio-Example-Models/', 'PEM:').replace('work-simucom/models/', 'work-simucom:')
    parts = src.split(',')
    if len(parts) > 1:
        src = parts[0] + ' + ' + os.path.basename(parts[1])
    sources[name] = src

CPU, HDD, DELAY = '_oro4gG3fEdy4YaaT-RYrLQ', '_BIjHoQ3KEdyouMqirZIhzQ', '_nvHX4KkREdyEA_b89s7q9w'

def features(d):
    txt = ''
    for f in sorted(os.listdir(d)):
        if f.endswith(('.repository', '.system', '.usagemodel', '.resourceenvironment', '.allocation')):
            txt += open(os.path.join(d, f), encoding='utf-8', errors='replace').read()
    types = set(re.findall(r'xsi:type="(?:seff|usagemodel|repository|subsystem|seff_performance):([A-Za-z]+)"', txt))
    out = []
    wl = [t for t in ('OpenWorkload', 'ClosedWorkload') if t in types]
    out += [w.replace('Workload', ' workload').lower() for w in wl]
    seff = ['BranchAction', 'LoopAction', 'CollectionIteratorAction', 'ForkAction', 'AcquireAction', 'ExternalCallAction',
            'SetVariableAction']
    out += [t for t in seff if t in types]
    if 'GuardedBranchTransition' in types:
        out.append('guarded branch')
    if 'ProbabilisticBranchTransition' in types:
        out.append('probabilistic branch')
    out += ['usage ' + t for t in ('Delay', 'Branch', 'Loop') if 'usagemodel:' + t in txt]
    if 'infrastructureCall__Action' in txt:
        out.append('InfrastructureCall')
    if 'resourceCall__Action' in txt:
        out.append('ResourceCall')
    if 'CompositeComponent' in types:
        out.append('CompositeComponent')
    if 'SubSystem' in types:
        out.append('SubSystem')
    if 'componentParameterUsage_ImplementationComponentType' in txt or 'configParameterUsages__AssemblyContext' in txt:
        out.append('component parameters')
    if 'passiveResource_BasicComponent' in txt:
        out.append('passive resources')
    if 'linkingResources__ResourceEnvironment' in txt:
        out.append('linking resource')
    res = set()
    for spec in re.findall(r'<activeResourceSpecifications_ResourceContainer.*?</activeResourceSpecifications_ResourceContainer>', txt, re.S):
        t = 'CPU' if CPU in spec else 'HDD' if HDD in spec else 'DELAY' if DELAY in spec else '?'
        p = re.search(r'Palladio\.resourcetype#(ProcessorSharing|FCFS|Delay)', spec)
        n = re.search(r'numberOfReplicas="(\d+)"', spec)
        res.add('%s/%s%s' % (t, {'ProcessorSharing': 'PS', 'FCFS': 'FCFS', 'Delay': 'Delay'}.get(p.group(1) if p else '', '?'),
                             ('x' + n.group(1)) if n and n.group(1) != '1' else ''))
    out.append(' '.join(sorted(res)))
    return ', '.join(x for x in out if x)

rows = []
for name in sorted(os.listdir(corpus)):
    d = os.path.join(corpus, name)
    rj = os.path.join(d, 'run.json')
    if not os.path.isfile(rj):
        continue
    cfg = json.load(open(rj))
    stop = []
    if cfg.get('max_measurements', -1) > 0:
        stop.append('%d meas' % cfg['max_measurements'])
    if cfg.get('max_sim_time', -1) > 0:
        stop.append('t=%d' % cfg['max_sim_time'])
    if cfg.get('simulate_throughput_of_linking_resources') is False:
        stop.append('no link throughput')
    ff = os.path.join(d, 'FEATURES.txt')
    feat = open(ff).read().strip() if os.path.isfile(ff) else features(d)
    src = sources.get(name, 'hand-made (`refsim gen`)')
    exp = os.path.join(d, 'expected')
    size = sum(os.path.getsize(os.path.join(exp, f)) for f in os.listdir(exp)) if os.path.isdir(exp) else 0
    r = runs.get(name, {})
    det = 'yes' if r.get('uniforms') == 0 else 'no'
    rows.append('| `%s` | %s | %s | seed %s, %s | %s | %s | %s | %s | %d KB |' % (
        name, feat, src if name.startswith('x_') else 'hand-made', cfg.get('seed'), ', '.join(stop), det,
        r.get('uniforms', '?'), r.get('meas', '?'), r.get('ms', '?'), size // 1024))

hdr = """# Model corpus

Every directory holds PCM model files (flat, relative hrefs), `run.json` (seed, stop conditions, network
flags; `docs/guide/formats.md` §5) and `expected/` (`trace.jsonl`, `tape.jsonl`, `measurements.csv` from
the patched reference; files > 256 KiB are stored as `.gz`, `gzip -n`).

- `h*`: hand-made models, one feature each. They are generated by `reference/refsim gen corpus`
  (`reference/src/refsim/corpus/HandMade.java`), and `FEATURES.txt` describes each one.
- `x_*`: external example models, imported by `reference/import-external.sh` from
  `reference/external-models.txt`. That list also records the skipped models and why.
  - Sources: `repos/Palladio-Example-Models` (`pem`), SimuLizar tests and examples (`sl`), Slingshot E2E
    (`ss`), and the work-simucom espresso and SimuLizar MediaStore copies.
  - These models get the refsim default monitors (`refsim.monitorrepository`: FeedThrough response time,
    active/passive resource metrics; see `Monitors.java`). The models' own monitors are not used.
- Regenerate expected outputs: `reference/regen-expected.sh`. Check them: `reference/regen-expected.sh --check`.
  Full determinism check: `reference/verify-determinism.sh`.

Columns:
- **deterministic w/o seed**: the run draws no random number, so the result does not depend on the seed.
- **uniforms**: number of draws.
- **meas**: number of recorded measurement tuples.
- **ms**: warm wall time of one run in the batch JVM, trace and tape included.
- **expected**: size on disk.

| model | features | source | run | deterministic w/o seed | uniforms | meas | ms | expected |
|---|---|---|---|---|---|---|---|---|
"""
open(os.path.join(corpus, 'INDEX.md'), 'w').write(hdr + '\n'.join(rows) + '\n')
print('wrote %s (%d models)' % (os.path.join(corpus, 'INDEX.md'), len(rows)))
