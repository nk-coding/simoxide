#!/usr/bin/env python3
"""Aggregates bench/compare raw logs (OUT/raw/{cold,warm,par}/*.log) into CSV files and Markdown tables.

  raw.csv        one row per simulation run (RESULT lines) of every benchmark process
  processes.csv  one row per benchmark process (wall time, peak RSS)
  summary.csv    per simulator x model x length: cold one-shot, warm per-run medians, rates, RSS
  parallel.csv   parallel throughput per configuration
  tables.md      the tables of docs/performance/comparison.md
usage: summarize.py OUT_DIR"""
import csv, glob, os, re, statistics, sys

OUT = sys.argv[1]
HERE = os.path.dirname(os.path.abspath(__file__))
lib = open(os.path.join(HERE, 'lib.sh')).read()
SIMTIME = {m: tuple(float(x) for x in v.split())
           for m, v in re.findall(r'\[(\w+)\]="([\d. ]+)"', lib.split('declare -A SIMTIME=(')[1].split(')')[0])}
MODELS = ['x_ss_minimal', 'x_espresso', 'x_ss_mediastore', 'x_sl_mediastore', 'h13_passive_contention', 'x_pem_fork']
SIMS = ['simoxide', 'refsim', 'simulizar', 'simulizar-osgi', 'simulizar-vt', 'slingshot', 'slingshot-osgi', 'simucom', 'eventsim']
COLD_ALIAS = {'simoxide-cli': 'simoxide'}
KV = re.compile(r'(\w+)=(\S+)')


def legacy(name):
    """Raw logs recorded before the rename to SimOxide use the label `pcmsim` (`pcmsim-cli`, ...)."""
    return re.sub(r'^pcmsim', 'simoxide', name)


def simtime(model, length):
    return SIMTIME[model][0 if length == 'short' else 1]


def num(x, default=float('nan')):
    try:
        return float(x)
    except (TypeError, ValueError):
        return default


def parse(path):
    """-> (results: list of dict, times: list of dict, meta: dict)"""
    res, times, meta, pars = [], [], {}, []
    with open(path, 'rb') as f:
        for raw in f:
            line = raw.decode('utf-8', 'replace').rstrip('\n')
            if line.startswith('RESULT '):
                res.append(dict(KV.findall(line)))
            elif line.startswith('TIME '):
                times.append(dict(KV.findall(line)))
            elif line.startswith('PAR '):
                pars.append(dict(KV.findall(line)))
            elif line.startswith('# GO_EPOCH_MS'):
                meta['go'] = int(line.split()[2])
                meta['mem_go'] = int(line.split()[4])
            elif line.startswith('# LOAD'):
                meta.setdefault('mem_start', int(line.split('MEMAVAIL_KB')[1].split()[0]))
            elif line.startswith('# END_EPOCH_MS'):
                meta['min_mem'] = int(line.split()[4])
            elif line.startswith('# KILLED'):
                meta['killed'] = True
    meta['par'] = pars
    return res, times, meta


def stem(path):
    parts = os.path.basename(path)[:-4].split('__')
    return parts + [''] * (4 - len(parts))


raw_rows, proc_rows = [], []
cold, warm, par = {}, {}, []
for phase in ('cold', 'warm', 'par'):
    for path in sorted(glob.glob(os.path.join(OUT, 'raw', phase, '*.log'))):
        cfg, model, length, rep = stem(path)
        cfg = legacy(cfg)
        res, times, meta = parse(path)
        T = simtime(model, length)
        for r in res:
            raw_rows.append({'phase': phase, 'config': cfg, 'sim': legacy(r.get('sim') or ''), 'model': model, 'len': length,
                             'sim_time_s': T, 'rep': rep, 'run_phase': r.get('phase'), 'run': r.get('run'),
                             'wall_ms': r.get('wall_ms'), 'sim_ms': r.get('sim_ms'), 'requests': r.get('requests'),
                             'mean_rt': r.get('mean_rt'), 'sim_end': r.get('sim_end'), 'events': r.get('events'),
                             'error': r.get('error', '')})
        for t in times:
            proc_rows.append({'phase': phase, 'config': cfg, 'model': model, 'len': length, 'rep': rep,
                              'wall_s': t.get('wall_s'), 'rss_kb': t.get('rss_kb'), 'user_s': t.get('user_s'),
                              'sys_s': t.get('sys_s'), 'rc': t.get('rc'), 'killed': meta.get('killed', False)})
        runs = [r for r in res if r.get('phase') == 'run']
        if phase == 'cold':
            sim = COLD_ALIAS.get(cfg, cfg)
            d = cold.setdefault((sim, model, length), {'wall': [], 'rss': [], 'req': [], 'rt': [], 'err': 0})
            for t in times:
                if t.get('rc') == '0' and not meta.get('killed'):
                    d['wall'].append(num(t['wall_s']))
                    d['rss'].append(num(t['rss_kb']))
                else:
                    d['err'] += 1
            for r in runs:
                d['req'].append(num(r.get('requests')))
                d['rt'].append(num(r.get('mean_rt')))
                d['err'] += 'error' in r
        elif phase == 'warm':
            ok = [r for r in runs if 'error' not in r]
            warm[(cfg, model, length)] = {
                'wall': [num(r['wall_ms']) for r in ok], 'sim': [num(r['sim_ms']) for r in ok],
                'req': [num(r['requests']) for r in ok], 'rt': [num(r['mean_rt']) for r in ok],
                'ev': [num(r['events']) for r in ok], 'end': [num(r['sim_end']) for r in ok],
                'err': len(runs) - len(ok) + (0 if times and times[-1].get('rc') == '0' else 1),
                'rss': num(times[-1]['rss_kb']) if times else float('nan')}
        else:
            ok = [r for r in runs if 'error' not in r]
            req = statistics.median([num(r['requests']) for r in ok]) if ok else float('nan')
            pars = meta['par']
            nruns = sum(int(p['runs']) for p in pars)
            if 'go' in meta:
                ends = [int(p['end_epoch_ms']) for p in pars]
                wall = (max(ends) - meta['go']) / 1000 if ends else float('nan')
                mem = (meta['mem_go'] - meta.get('min_mem', meta['mem_go'])) / 1024 ** 2 \
                    + (meta['mem_start'] - meta['mem_go']) / 1024 ** 2
                workers = len(pars)
                kind = 'isolated class loaders' if 'iso' in cfg else 'processes'
            else:
                wall = sum(num(p['wall_ms']) for p in pars) / 1000
                mem = num(times[-1]['rss_kb']) / 1024 ** 2 if times else float('nan')
                workers = int(pars[0]['threads']) if pars else 0
                kind = 'batch API threads' if 'batch' in cfg else 'threads'
            par.append({'config': cfg, 'model': model, 'len': length, 'sim_time_s': simtime(model, length),
                        'kind': kind, 'workers': workers, 'runs': nruns, 'failed_runs': len(runs) - len(ok),
                        'wall_s': round(wall, 3), 'runs_per_s': round(nruns / wall, 3) if wall else '',
                        'requests_per_run': req, 'requests_per_s': round(req * nruns / wall, 1) if wall else '',
                        'mem_mb': round(mem * 1024) if mem == mem else '', 'killed': meta.get('killed', False)})


def write(name, rows):
    if not rows:
        return
    with open(os.path.join(OUT, name), 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)


def med(v):
    v = [x for x in v if x == x]
    return statistics.median(v) if v else float('nan')


summary = []
for model in MODELS:
    for length in ('short', 'long'):
        T = simtime(model, length)
        for sim in SIMS:
            c, w = cold.get((sim, model, length)), warm.get((sim, model, length))
            if not c and not w:
                continue
            row = {'sim': sim, 'model': model, 'len': length, 'sim_time_s': T}
            if c:
                row.update(cold_wall_s=med(c['wall']), cold_rss_mb=med(c['rss']) / 1024, cold_reps=len(c['wall']),
                           cold_errors=c['err'])
            if w and w['wall']:
                wall = med(w['wall'])
                req = med(w['req'])
                ev = med(w['ev'])
                row.update(warm_runs=len(w['wall']), warm_wall_ms=wall, warm_wall_min_ms=min(w['wall']),
                           warm_wall_max_ms=max(w['wall']), warm_sim_ms=med(w['sim']), requests=req,
                           mean_rt=med(w['rt']), events=ev if ev >= 0 else '',
                           requests_per_s=req / wall * 1000 if req >= 0 else '',
                           simsec_per_s=T / wall * 1000, events_per_s=ev / wall * 1000 if ev >= 0 else '',
                           warm_rss_mb=w['rss'] / 1024, warm_errors=w['err'])
            elif w:
                row.update(warm_errors=w['err'])
            summary.append(row)

write('raw.csv', raw_rows)
write('processes.csv', proc_rows)
keys = []
for r in summary:
    keys += [k for k in r if k not in keys]
summary = [{k: r.get(k, '') for k in keys} for r in summary]
write('summary.csv', summary)
write('parallel.csv', par)


# ---------------------------------------------------------------- Markdown
def fmt(x, digits=3):
    if x == '' or x is None or (isinstance(x, float) and x != x):
        return '–'
    x = float(x)
    if x == 0:
        return '0'
    a = abs(x)
    if a >= 1e9:
        return f'{x / 1e6:,.0f} M'.replace(',', ' ')
    if a >= 1e6:
        return f'{x / 1e6:.3g} M'
    if a >= 100:
        return f'{x:,.0f}'.replace(',', ' ')
    return f'{x:.{digits}g}'


def speed(p, o):
    try:
        if not (float(o) == float(o) and float(p) == float(p)):
            return '–'
        return f'{float(o) / float(p):,.0f}x'.replace(',', ' ') if float(o) / float(p) >= 10 else f'{float(o) / float(p):.1f}x'
    except (TypeError, ValueError, ZeroDivisionError):
        return '–'


NAMES = {'simoxide': 'SimOxide', 'refsim': 'refsim (patched)', 'simulizar': 'SimuLizar 5.2.2 stock',
         'simulizar-vt': 'SimuLizar + VT patch', 'simulizar-osgi': 'SimuLizar stock, OSGi (cold only)',
         'slingshot-osgi': 'Slingshot, OSGi (cold only)', 'slingshot': 'Slingshot', 'simucom': 'SimuCom 5.2.2*',
         'eventsim': 'EventSim 5.1 (archived)'}
md = []
by = {(r['sim'], r['model'], r['len']): r for r in summary}
def ms(x):
    if x == '' or x != x:
        return '–'
    x = float(x)
    return f'{x / 1000:.3g} s' if x >= 1000 else f'{x:.3g} ms'


MAIN = ['simoxide', 'refsim', 'simulizar', 'simulizar-vt', 'slingshot', 'simucom', 'eventsim']
for kind, key, title in (('warm', 'warm_wall_ms', 'Warm per-run wall time (median; in brackets: SimOxide speed-up)'),
                         ('cold', 'cold_wall_s', 'Cold one-shot wall time, process start to results (median of 3; SimOxide speed-up)')):
    md.append(f'### {title}\n')
    cols = MAIN + (['simulizar-osgi', 'slingshot-osgi'] if kind == 'cold' else [])
    md.append('| model | simulated s | ' + ' | '.join(NAMES[c] for c in cols) + ' |')
    md.append('|---|---|' + '---|' * len(cols))
    for model in MODELS:
        for length in ('short', 'long'):
            p = by.get(('simoxide', model, length))
            if not p:
                continue
            cells = []
            for c in cols:
                r = by.get((c, model, length), {})
                v = r.get(key, '')
                if v == '' or v != v:
                    cells.append('–' if c != 'slingshot' or model != 'x_pem_fork' else 'fails')
                    continue
                v = float(v) if kind == 'warm' else float(v) * 1000
                base = float(p[key]) if kind == 'warm' else float(p[key]) * 1000
                cells.append(ms(v) + ('' if c == 'simoxide' else f' ({speed(base, v)})'))
            md.append(f'| {model} {length} | {fmt(simtime(model, length))} | ' + ' | '.join(cells) + ' |')
    md.append('')

md.append('### Per-run times (warm = median in a long-lived process; cold = one process, start to results)\n')
for model in MODELS:
    for length in ('short', 'long'):
        rows = [by[(s, model, length)] for s in SIMS if (s, model, length) in by]
        if not rows:
            continue
        p = by.get(('simoxide', model, length), {})
        md.append(f'**{model}, {length}: {fmt(simtime(model, length))} s simulated**\n')
        md.append('| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | '
                  'peak RSS warm / cold | SimOxide speed-up warm / cold |')
        md.append('|---|---|---|---|---|---|---|---|---|')
        for r in rows:
            wm = r.get('warm_wall_ms', '')
            md.append(f"| {NAMES[r['sim']]} | {fmt(wm)} ms | {fmt(r.get('requests', ''))} | "
                      f"{fmt(r.get('requests_per_s', ''))} | {fmt(r.get('simsec_per_s', ''))} | "
                      f"{fmt(r.get('events_per_s', ''))} | {fmt(r.get('cold_wall_s', ''))} s | "
                      f"{fmt(r.get('warm_rss_mb', ''))} / {fmt(r.get('cold_rss_mb', ''))} MB | "
                      + ('–' if r['sim'] == 'simoxide' else
                         f"{speed(p.get('warm_wall_ms'), wm)} / {speed(p.get('cold_wall_s'), r.get('cold_wall_s'))}")
                      + ' |')
        md.append('')

md.append('### Sanity check: usage-scenario response time (warm runs, seed 1)\n')
SAN = [x for x in SIMS if not x.endswith('-osgi')]
md.append('| model | length | ' + ' | '.join(NAMES[s] for s in SAN) + ' |')
md.append('|---|---|' + '---|' * len(SAN))
for model in MODELS:
    for length in ('short', 'long'):
        cells = []
        for s in SAN:
            r = by.get((s, model, length))
            if not r or r.get('requests', '') == '':
                cells.append('–')
            else:
                rt = r.get('mean_rt', '')
                cells.append(f"{f'{float(rt):.7g}' if rt == rt and rt != '' else 'n/a'} (n={fmt(r['requests'])})")
        md.append(f'| {model} | {length} | ' + ' | '.join(cells) + ' |')
md.append('')

md.append('### Parallel throughput (all 22 cores)\n')
md.append('| model | length | configuration | workers | runs | wall s | runs/s | requests/s | memory MB |')
md.append('|---|---|---|---|---|---|---|---|---|')
for r in sorted(par, key=lambda r: (MODELS.index(r['model']), r['len'], -num(r['runs_per_s'], 0))):
    md.append(f"| {r['model']} | {r['len']} | {r['config']} ({r['kind']}) | {r['workers']} | {r['runs']} | "
              f"{fmt(r['wall_s'])} | {fmt(r['runs_per_s'])} | {fmt(r['requests_per_s'])} | {fmt(r['mem_mb'])} |"
              + (' KILLED' if r['killed'] else '') + (f" {r['failed_runs']} failed" if r['failed_runs'] else ''))
open(os.path.join(OUT, 'tables.md'), 'w').write('\n'.join(md) + '\n')
print(f'{len(raw_rows)} runs, {len(proc_rows)} processes -> {OUT}/{{raw,processes,summary,parallel}}.csv, tables.md')
