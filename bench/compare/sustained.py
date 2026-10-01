#!/usr/bin/env python3
"""Aggregates the sustained phases (OUT/raw/{sus1,susN,longrun}) of bench/compare.

  sus_runs.csv      every measured run: completion time since the measured start, per-run wall, requests, error
  sus_windows.csv   throughput per 30 s window (runs/s, simulated requests/s, failures)
  sus_samples.csv   process-tree RSS and thread count every 2 s
  sus_summary.csv   per configuration: steady-state throughput, warm-up time, p50/p99/max latency, GC pause
                    share, RSS and threads (after start-up / max / end), failures, degradation
  longrun.csv       one very long run per simulator (+ longrun_progress.csv: refsim's in-run progress)
  tables_sustained.md
usage: sustained.py OUT_DIR"""
import csv, glob, os, re, statistics, sys

OUT = sys.argv[1]
WIN = 30.0
KV = re.compile(r'(\w+)=(\S+)')


def legacy(name):
    """Raw logs recorded before the rename to SimOxide use the label `pcmsim` (`pcmsim-threads-22`, ...)."""
    return re.sub(r'^pcmsim', 'simoxide', name)


def num(x, d=float('nan')):
    try:
        return float(x)
    except (TypeError, ValueError):
        return d


def pct(v, q):
    if not v:
        return float('nan')
    v = sorted(v)
    return v[min(len(v) - 1, max(0, int(round(q * len(v) + 0.5)) - 1))]


def read_log(path):
    res, prog, meta = [], [], {'par': [], 'times': []}
    for raw in open(path, 'rb'):
        line = raw.decode('utf-8', 'replace').rstrip('\n')
        if line.startswith('RESULT '):
            res.append(dict(KV.findall(line)))
        elif line.startswith('PROGRESS '):
            prog.append(dict(KV.findall(line)))
        elif line.startswith('PAR '):
            meta['par'].append(dict(KV.findall(line)))
        elif line.startswith('TIME '):
            meta['times'].append(dict(KV.findall(line)))
        elif line.startswith('# GO_EPOCH_MS'):
            meta['go'] = int(line.split()[2])
        elif line.startswith('# CMD'):
            m = re.search(r'duration=([\d.]+)', line)
            if m and 'duration' not in meta:
                meta['duration'] = float(m.group(1))
        elif line.startswith('# LOAD'):
            m = re.search(r'START_EPOCH_MS (\d+)', line)
            if m:
                meta.setdefault('start', int(m.group(1)))
        elif line.startswith('# KILLED'):
            meta['killed'] = True
    return res, prog, meta


def gc_share(path):
    """(total pause ms, total JVM lifetime ms, number of JVMs) from -Xlog:gc files."""
    pause = life = 0.0
    n = 0
    for f in glob.glob(path + '.gc.*.txt'):
        n += 1
        last = 0.0
        for line in open(f, errors='replace'):
            m = re.match(r'\[(\d+)ms\]', line)
            if m:
                last = float(m.group(1))
            if ' Pause ' in line:
                m = re.search(r'([\d.]+)ms\s*$', line)
                if m:
                    pause += float(m.group(1))
        life += last
    return pause, life, n


def samples(path):
    f = path + '.rss.csv'
    if not os.path.exists(f):
        return []
    return [{k: num(v) for k, v in r.items()} for r in csv.DictReader(open(f))]


runs_rows, win_rows, sample_rows, summary, longrows, progrows = [], [], [], [], [], []
for phase in ('sus1', 'susN'):
    for path in sorted(glob.glob(os.path.join(OUT, 'raw', phase, '*.log'))):
        cfg, model, T = os.path.basename(path)[:-4].split('__')
        cfg = legacy(cfg)
        T = float(T)
        res, prog, meta = read_log(path)
        D = meta.get('duration', float('nan'))
        runs = [r for r in res if r.get('phase') == 'run']
        # completion time since the measured start
        base = meta.get('go')
        if base is None:
            eps = [num(r['epoch_ms']) - num(r['t_end_ms']) for r in runs if 'epoch_ms' in r]
            base = min(eps) if eps else None
        pts = []
        for r in runs:
            if phase == 'susN' and 'epoch_ms' in r and base is not None:
                t = (num(r['epoch_ms']) - base) / 1000
            else:
                t = num(r.get('t_end_ms')) / 1000
            err = 'error' in r or num(r.get('requests'), 0) <= 0
            pts.append((t, num(r.get('wall_ms')), num(r.get('requests'), 0), err))
            runs_rows.append({'phase': phase, 'config': cfg, 'model': model, 'sim_time_s': T, 't_s': round(t, 3),
                              'wall_ms': r.get('wall_ms'), 'requests': r.get('requests'),
                              'error': r.get('error', '') or ('empty result' if err else '')})
        pts.sort()
        span = D if D == D else (pts[-1][0] if pts else 0)
        nwin = max(1, int(span // WIN))
        wins = []
        for w in range(nwin):
            a, b = w * WIN, (w + 1) * WIN
            inw = [p for p in pts if a <= p[0] < b]
            ok = [p for p in inw if not p[3]]
            wins.append((a, len(ok) / WIN, sum(p[2] for p in ok) / WIN, len(inw) - len(ok)))
            win_rows.append({'phase': phase, 'config': cfg, 'model': model, 'window_start_s': a,
                             'runs_per_s': round(len(ok) / WIN, 4), 'requests_per_s': round(sum(p[2] for p in ok) / WIN, 1),
                             'failures': len(inw) - len(ok)})
        smp = samples(path)
        for s in smp:
            sample_rows.append({'phase': phase, 'config': cfg, 'model': model, **s})
        half = [w for w in wins if w[0] >= span / 2]
        steady = statistics.mean(w[1] for w in half) if half else float('nan')
        steady_req = statistics.mean(w[2] for w in half) if half else float('nan')
        # warm-up: start of the first run from which the mean of 5 consecutive per-run times stays within
        # 10 % of the steady-state median (second half)  [mean, so one slow run counts]
        okp = [p for p in pts if not p[3] and p[1] >= 0]
        sp50 = pct([p[1] for p in okp if p[0] >= span / 2], .5)
        warm = float('nan')
        for i in range(len(okp)):
            if statistics.mean(p[1] for p in okp[i:i + 5]) <= 1.1 * sp50:
                warm = max(0.0, okp[i][0] - okp[i][1] / 1000)
                break
        third = max(1, len(wins) // 3)
        mid = [w[1] for w in wins[third:2 * third]]
        last = [w[1] for w in wins[-third:]]
        degr = (statistics.mean(last) / statistics.mean(mid)) if mid and statistics.mean(mid) > 0 else float('nan')
        walls = [p[1] for p in pts if not p[3] and p[1] >= 0]
        swalls = [p[1] for p in pts if not p[3] and p[1] >= 0 and p[0] >= span / 2]
        pause, life, njvm = gc_share(path)
        after = [s for s in smp if s['t_s'] >= 20] or smp
        workers = len(meta['par']) or 1
        summary.append({
            'phase': phase, 'config': cfg, 'model': model, 'sim_time_s': T, 'duration_s': D,
            'workers': workers if phase == 'susN' else 1, 'runs': len(pts), 'failures': sum(p[3] for p in pts),
            'steady_runs_per_s': round(steady, 3), 'steady_requests_per_s': round(steady_req, 1),
            'first_window_runs_per_s': round(wins[0][1], 3) if wins else '',
            'peak_window_runs_per_s': round(max(w[1] for w in wins), 3) if wins else '',
            'last_window_runs_per_s': round(wins[-1][1], 3) if wins else '',
            'warmup_s': warm, 'degradation_last_vs_middle': round(degr, 3),
            'first_run_ms': round(walls[0], 1) if walls else '',
            'p50_ms': round(pct(walls, .5), 2), 'p99_ms': round(pct(walls, .99), 2), 'max_ms': round(max(walls), 1) if walls else '',
            'steady_p50_ms': round(pct(swalls, .5), 2), 'steady_p99_ms': round(pct(swalls, .99), 2),
            'gc_pause_share': round(pause / life, 4) if life else '', 'gc_jvms': njvm,
            'rss_mb_start': round(after[0]['rss_kb'] / 1024) if after else '',
            'rss_mb_max': round(max(s['rss_kb'] for s in smp) / 1024) if smp else '',
            'rss_mb_end': round(smp[-1]['rss_kb'] / 1024) if smp else '',
            'threads_start': int(after[0]['threads']) if after else '',
            'threads_max': int(max(s['threads'] for s in smp)) if smp else '',
            'threads_end': int(smp[-1]['threads']) if smp else '',
            'min_mem_avail_gb': round(min(s['mem_avail_kb'] for s in smp) / 1024 ** 2, 1) if smp else '',
            'killed': meta.get('killed', False)})

# very long single runs
warm = {}
sp = os.path.join(OUT, 'summary.csv')
if os.path.exists(sp):
    for r in csv.DictReader(open(sp)):
        if r['len'] == 'long' and r.get('warm_wall_ms'):
            warm[(legacy(r['sim']), r['model'])] = float(r['simsec_per_s'])
for path in sorted(glob.glob(os.path.join(OUT, 'raw', 'longrun', '*.log'))):
    sim, model, T = os.path.basename(path)[:-4].split('__')
    sim = legacy(sim)
    T = float(T)
    res, prog, meta = read_log(path)
    runs = [r for r in res if r.get('phase') == 'run']
    t = meta['times'][-1] if meta['times'] else {}
    smp = samples(path)
    r = runs[0] if runs else {}
    finished = bool(r) and 'error' not in r
    simms = num(r.get('wall_ms')) if finished else float('nan')
    rate = T / (simms / 1000) if finished else float('nan')
    for p in prog:
        progrows.append({'sim': sim, 'model': model, 'sim_time_s': T, 't_s': num(p['t_ms']) / 1000,
                         'sim_time': p.get('sim_time'), 'measurements': p.get('measurements'), 'heap_mb': p.get('heap_mb')})
    # refsim in-run rate: simulated seconds per wall second, first vs last minute
    inrun = ''
    pr = [(num(p['t_ms']) / 1000, num(p.get('sim_time'))) for p in prog if num(p.get('sim_time'), -1) > 0]
    if len(pr) >= 4:
        def rate_between(a, b):
            seg = [x for x in pr if a <= x[0] <= b]
            return (seg[-1][1] - seg[0][1]) / (seg[-1][0] - seg[0][0]) if len(seg) >= 2 and seg[-1][0] > seg[0][0] else float('nan')
        end = pr[-1][0]
        inrun = f"{rate_between(pr[0][0], pr[0][0] + 60):.0f} -> {rate_between(end - 60, end):.0f}"
    pause, life, _ = gc_share(path)
    longrows.append({
        'sim': sim, 'model': model, 'sim_time_s': T, 'finished': finished, 'timeout': 'timeout' in t,
        'process_wall_s': num(t.get('wall_s')), 'run_wall_s': round(simms / 1000, 3) if finished else '',
        'requests': r.get('requests', ''), 'mean_rt': r.get('mean_rt', ''),
        'requests_per_s': round(num(r.get('requests')) / (simms / 1000), 1) if finished else '',
        'simsec_per_s': round(rate, 1) if finished else '',
        'warm_long_simsec_per_s': round(warm.get((sim, model), float('nan')), 1),
        'ratio_vs_warm_long': round(rate / warm[(sim, model)], 3) if finished and (sim, model) in warm else '',
        'refsim_inrun_simsec_per_s_first_vs_last_min': inrun,
        'rss_mb_max': round(max([s['rss_kb'] for s in smp] + [num(t.get('rss_kb'), 0)]) / 1024),
        'gc_pause_share': round(pause / life, 4) if life else '',
        'rss_mb_at_20s': next((round(x['rss_kb'] / 1024) for x in smp if x['t_s'] >= 20), ''),
        'rss_mb_before_end': round(smp[-2]['rss_kb'] / 1024) if len(smp) >= 2 else ''})


def write(name, rows):
    if rows:
        keys = []
        for r in rows:
            keys += [k for k in r if k not in keys]
        with open(os.path.join(OUT, name), 'w', newline='') as f:
            w = csv.DictWriter(f, fieldnames=keys)
            w.writeheader()
            w.writerows(rows)


write('sus_runs.csv', runs_rows)
write('sus_windows.csv', win_rows)
write('sus_samples.csv', sample_rows)
write('sus_summary.csv', summary)
write('longrun.csv', longrows)
write('longrun_progress.csv', progrows)


def f(x, d=3):
    if x == '' or x is None or (isinstance(x, float) and x != x):
        return '–'
    x = float(x)
    if abs(x) >= 100:
        return f'{x:,.0f}'.replace(',', ' ')
    return f'{x:.{d}g}'


md = []
for phase, title in (('sus1', 'Sustained, one core'), ('susN', 'Sustained, all 22 cores')):
    rows = [r for r in summary if r['phase'] == phase]
    if not rows:
        continue
    md.append(f'### {title}\n')
    for model in dict.fromkeys(r['model'] for r in rows):
        # simoxide-batch is left out of the table: the driver computes the summaries of each run_batch chunk on
        # one thread, which idles the other cores (see sus_summary.csv); the reload-threads mode is the fair one
        rs = [r for r in rows if r['model'] == model and not r['config'].startswith('simoxide-batch')]
        p = next((r for r in rs if r['config'] == 'simoxide' or r['config'].startswith('simoxide-threads')), None)
        md.append(f"**{model}, {f(rs[0]['sim_time_s'])} s simulated per run** (config: wall duration in brackets)\n")
        md.append('| config | runs (failed) | steady runs/s | steady requests/s | SimOxide speed-up | warm-up s | '
                  '30-s windows: first / peak / last runs/s | last/middle third | p50 / p99 / max ms | GC pause | RSS MB start / max / end | threads start / max / end |')
        md.append('|---|---|---|---|---|---|---|---|---|---|---|---|')
        for r in rs:
            sp = (f"{p['steady_runs_per_s'] / r['steady_runs_per_s']:,.0f}x".replace(',', ' ')
                  if p and r is not p and r['steady_runs_per_s'] else '–')
            md.append(f"| {r['config']} ({f(r['duration_s'])} s) | {r['runs']} ({r['failures']}) | {f(r['steady_runs_per_s'])} | "
                      f"{f(r['steady_requests_per_s'])} | {sp} | {f(r['warmup_s'])} | "
                      f"{f(r['first_window_runs_per_s'])} / {f(r['peak_window_runs_per_s'])} / {f(r['last_window_runs_per_s'])} | "
                      f"{f(r['degradation_last_vs_middle'])} | "
                      f"{f(r['steady_p50_ms'])} / {f(r['steady_p99_ms'])} / {f(r['max_ms'])} | "
                      f"{f(100 * r['gc_pause_share']) + ' %' if r['gc_pause_share'] != '' else '–'} | "
                      f"{f(r['rss_mb_start'])} / {f(r['rss_mb_max'])} / {f(r['rss_mb_end'])} | "
                      f"{r['threads_start']} / {r['threads_max']} / {r['threads_end']} |"
                      + (' KILLED' if r['killed'] else ''))
        md.append('')
if longrows:
    md.append('### One very long run\n')
    md.append('| simulator | simulated s | finished | run wall s | requests | sim s / s | vs warm "long" | '
              'refsim in-run sim s/s (first → last minute) | RSS MB at 20 s → end → peak | GC pause |')
    md.append('|---|---|---|---|---|---|---|---|---|---|')
    for r in longrows:
        md.append(f"| {r['sim']} | {f(r['sim_time_s'])} | {'yes' if r['finished'] else ('no (300 s cap)' if r['timeout'] else 'no')} | "
                  f"{f(r['run_wall_s'])} | {r['requests'] or '–'} | {f(r['simsec_per_s'])} | {f(r['ratio_vs_warm_long'])} | "
                  f"{r['refsim_inrun_simsec_per_s_first_vs_last_min'] or '–'} | {f(r['rss_mb_at_20s'])} → {f(r['rss_mb_before_end'])} → {f(r['rss_mb_max'])} | "
                  f"{f(100 * r['gc_pause_share']) + ' %' if r['gc_pause_share'] != '' else '–'} |")
open(os.path.join(OUT, 'tables_sustained.md'), 'w').write('\n'.join(md) + '\n')
print(f'sustained: {len(summary)} configs, {len(runs_rows)} runs, {len(longrows)} long runs')
