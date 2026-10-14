#!/usr/bin/env python3
"""Numbers for the hand-written parts of the website from an SSJ session (bench/compare/SSJ.md).

Reads OUT/{summary,sus_summary}.csv and OUT/raw (written by ssj/run.sh) and fills the {{PLACEHOLDERS}} of
ssj/docs-draft.patch (README.md, docs/index.md, docs/performance/{index,comparison}.md,
docs/development/benchmarking.md). Writes
  OUT/docs-numbers.md   every placeholder with its value and where it comes from
  OUT/docs-ssj.patch    the draft with the values filled in (git apply; review the wording first)
Placeholders without data stay as they are. The generated tables (OUT/tables*.md) are included by
comparison.md directly.
usage: docs-numbers.py OUT_DIR"""
import csv, glob, os, re, statistics, sys, time

OUT = sys.argv[1]
HERE = os.path.dirname(os.path.abspath(__file__))
MODELS = ['x_ss_minimal', 'x_espresso', 'x_ss_mediastore', 'x_sl_mediastore', 'h13_passive_contention', 'x_pem_fork']


def rows(name):
    p = os.path.join(OUT, name)
    return list(csv.DictReader(open(p))) if os.path.exists(p) else []


def num(x):
    try:
        return float(x)
    except (TypeError, ValueError):
        return float('nan')


def g(x):  # 3 significant digits, thin-space thousands like the docs (2 420, 0.53)
    if x != x:
        return '–'
    if abs(x) >= 100:
        return f'{x:,.0f}'.replace(',', ' ')
    return f'{x:.3g}'


summ = {(r['sim'], r['model'], r['len']): r for r in rows('summary.csv')}
sus = rows('sus_summary.csv')
V, SRC = {}, {}


def put(k, v, src):
    if v is not None:
        V[k], SRC[k] = v, src


def ratios(a, b, key, models=MODELS):
    """key(a) / key(b) over the standard models and both lengths."""
    out = []
    for m in models:
        for ln in ('short', 'long'):
            x, y = num(summ.get((a, m, ln), {}).get(key)), num(summ.get((b, m, ln), {}).get(key))
            if x == x and y == y and y > 0:
                out.append(x / y)
    return out


def rng(v, unit=''):
    return f'{g(min(v))}{unit} to {g(max(v))}{unit}' if v else None


# session date and machine
epochs = []
for f in glob.glob(os.path.join(OUT, 'raw', '*', '*.log')):
    with open(f, 'rb') as fh:
        for line in fh:
            m = re.search(rb'START_EPOCH_MS (\d+)', line)
            if m:
                epochs.append(int(m.group(1)))
                break
if epochs:
    d0, d1 = (time.strftime('%Y-%m-%d', time.localtime(e / 1000)) for e in (min(epochs), max(epochs)))
    cores = os.cpu_count()
    mem = int(open('/proc/meminfo').read().split('MemTotal:')[1].split()[0]) / 1024 ** 2
    put('SSJ_SESSION', f"{d0}{'' if d0 == d1 else ' to ' + d1}, {cores} cores, {mem:.0f} GB",
        'raw/*/*.log START_EPOCH_MS; cores and RAM of the machine running this script')

for sim, key in (('simulizar', 'SL'), ('simucom', 'SC')):
    for eng, s in (('DESMOJ', sim), ('SSJ', sim + '-ssj')):
        sp = ratios(s, 'simoxide', 'warm_wall_ms')
        put(f'{key}_{eng}_WARM_SPEEDUP', rng(sp), f'summary.csv warm_wall_ms {s} / simoxide, 6 models x 2 lengths')
        put(f'{key}_{eng}_WARM_SPEEDUP_X', rng(sp, '×'), f'same as {key}_{eng}_WARM_SPEEDUP')
    cold = [num(r['cold_wall_s']) for (s, m, ln), r in summ.items()
            if s == sim + '-ssj' and m in MODELS and num(r.get('cold_wall_s')) == num(r.get('cold_wall_s'))]
    put(f'{key}_SSJ_COLD', rng(cold, ' s').replace(' s to', ' to') if cold else None,
        f'summary.csv cold_wall_s of {sim}-ssj')
    r = ratios(sim + '-ssj', sim, 'warm_wall_ms')
    if r:
        put(f'{key}_SSJ_VS_DESMOJ', f'{g(100 * min(r))} to {g(100 * max(r))} % (median {g(100 * statistics.median(r))} %)',
            f'summary.csv warm_wall_ms {sim}-ssj / {sim}')
    one = [x for x in sus if x['phase'] == 'sus1' and x['config'] == sim + '-ssj' and x['model'] == 'x_ss_mediastore']
    if one:
        put(f'{key}_SSJ_ONECORE', f"{g(num(one[0]['steady_runs_per_s']))} runs/s",
            'sus_summary.csv sus1 x_ss_mediastore steady_runs_per_s')

alls = [x for x in sus if x['phase'] == 'susN' and x['config'].startswith('simulizar-ssj-')]
if alls:
    parts = []
    for x in sorted(alls, key=lambda x: -num(x['steady_runs_per_s'])):
        how = 'in one JVM' if '-threads-' in x['config'] else f"{x['config'].rsplit('-', 1)[1]} processes"
        parts.append(f"{g(num(x['steady_runs_per_s']))} runs/s at {g(num(x['rss_mb_max']) / 1024)} GB ({how})"
                     + (', killed' if x.get('killed') == 'True' else '')
                     + (f", {x['failures']} failed runs" if x['failures'] not in ('0', '') else ''))
    put('SL_SSJ_ALLCORES', '; '.join(parts), 'sus_summary.csv susN simulizar-ssj-*')

sl = ratios('simulizar', 'simoxide', 'warm_wall_ms') + ratios('simulizar-ssj', 'simoxide', 'warm_wall_ms')
put('SL_HEADLINE_RANGE', rng(sl), 'min and max of SL_DESMOJ_WARM_SPEEDUP and SL_SSJ_WARM_SPEEDUP')

c = [f"SimuLizar {g(100 * min(r))} to {g(100 * max(r))} %" for r in [ratios('simulizar-ssj', 'simulizar', 'cold_wall_s')] if r]
c += [f"SimuCom {g(100 * min(r))} to {g(100 * max(r))} %" for r in [ratios('simucom-ssj', 'simucom', 'cold_wall_s')] if r]
put('SSJ_COLD_VS_DESMOJ', ', '.join(c) or None, 'summary.csv cold_wall_s ssj / desmoj')

cons = []
for sim, name in (('simulizar', 'SimuLizar'), ('simucom', 'SimuCom')):
    drt, dreq = [], []
    for m in MODELS:
        for ln in ('short', 'long'):
            a, b = summ.get((sim + '-ssj', m, ln)), summ.get((sim, m, ln))
            if a and b and num(b.get('mean_rt')) > 0 and num(b.get('requests')) > 0:
                drt.append(abs(num(a['mean_rt']) / num(b['mean_rt']) - 1))
                dreq.append(abs(num(a['requests']) / num(b['requests']) - 1))
    if drt:
        cons.append(f"{name} on SSJ: mean response time within {g(100 * max(drt))} % and request count within "
                    f"{g(100 * max(dreq))} % of DESMO-J (seed 1, every model and length)")
put('SSJ_CONSISTENCY', '; '.join(cons) or None, 'summary.csv mean_rt and requests, ssj vs desmoj')

stab = []
for sim, name in (('simulizar-ssj', 'SimuLizar'), ('simucom-ssj', 'SimuCom')):
    for x in sus:
        if x['phase'] == 'sus1' and x['config'] == sim and x['model'] == 'x_ss_mediastore':
            stab.append(f"{name}: {x['runs']} runs ({x['failures']} failed), threads {x['threads_start']} → "
                        f"{x['threads_end']}, RSS {x['rss_mb_start']} → {x['rss_mb_end']} MB, last/middle third "
                        f"{g(num(x['degradation_last_vs_middle']))}")
put('SSJ_STABILITY', '; '.join(stab) or None, 'sus_summary.csv sus1 x_ss_mediastore (review the wording)')
eff = []
for x in sus:
    if x['phase'] == 'sus1' and x['config'] in ('simulizar-ssj', 'simucom-ssj', 'simulizar', 'simucom'):
        eff.append(f"{x['config']} on {x['model']}: {x['threads_start']} → {x['threads_max']} threads, "
                   f"RSS max {x['rss_mb_max']} MB in {x['runs']} runs")
put('SSJ_STOP_EFFECT', ('Sustained one-core runs: ' + '; '.join(eff) + '.') if eff else None,
    'sus_summary.csv sus1 threads and RSS (review: is the SSJ growth the leak?)')

# fill the draft
draft = open(os.path.join(HERE, 'docs-draft.patch')).read()
missing = sorted(set(re.findall(r'\{\{(\w+)\}\}', draft)) - set(V))
filled = re.sub(r'\{\{(\w+)\}\}', lambda m: V.get(m.group(1), m.group(0)), draft)
open(os.path.join(OUT, 'docs-ssj.patch'), 'w').write(filled)
with open(os.path.join(OUT, 'docs-numbers.md'), 'w') as f:
    f.write('| placeholder | value | source |\n|---|---|---|\n')
    for k in sorted(set(V) | set(missing)):
        f.write(f"| `{k}` | {V.get(k, '**missing**')} | {SRC.get(k, '')} |\n")
for k in sorted(V):
    print(f'{k:28} {V[k]}')
print(f"missing: {', '.join(missing) or 'none'}\n-> {OUT}/docs-numbers.md, {OUT}/docs-ssj.patch")
