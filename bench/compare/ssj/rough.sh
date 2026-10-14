#!/bin/bash
# Rough single-threaded comparison (SSJ.md, "Rough single-threaded comparison"): SimOxide vs stock SimuLizar 5.2.2 on
# DESMO-J and on SSJ, on the website's six models, with fewer repetitions than ssj/run.sh, pinned to one core.
# Not the website benchmark: no wait for a quiet machine, shared VM.
#
#   bench/compare/ssj/rough.sh [run] [report]        (default: both; "run" resumes, finished processes are kept)
# Env: OUT (ssj/results/rough-<date>), CORE (4), ROUNDS (2), MODELS (all six), SIMS (simoxide simulizar simulizar-ssj)
#
# Per round and model, the simulators run one after another (interleaved): warm short, warm long, cold short.
# Warm processes (one JVM): short 20 warm-up + 10 measured runs, long 1 + 3 (SimOxide: 50 + 200, 3 + 20).
# Cold: one process per run (process start -> results), short only. Every round appends to the same warm log.
set -u
source "$(cd "$(dirname "$0")" && pwd)/common.sh"
OUT=${OUT:-$SSJ_HOME/results/rough-$(date +%F)}
CORE=${CORE:-4}
ROUNDS=${ROUNDS:-2}
MODELS=${MODELS:-$ALL_MODELS}
SIMS=${SIMS:-"simoxide simulizar simulizar-ssj"}
export JOPTS="-Xmx4g -XX:ActiveProcessorCount=1"   # one CPU for the JVM's ergonomics (GC, JIT threads)
export MEM_FLOOR_KB=${MEM_FLOOR_KB:-2000000}
PHASES=${*:-run report}
mkdir -p "$OUT/raw/warm" "$OUT/raw/cold"
LOADLOG=$OUT/load.log

loadinfo() { echo "load=$(cut -d' ' -f1-4 /proc/loadavg | tr ' ' ,) memavail_kb=$(mem_avail_kb)"; }

# waits at most 30 s for <= 2 runnable tasks (the other agents' builds are not ours to stop)
wait_brief() {
  local i=0
  while [ $i -lt 30 ]; do
    [ "$(cut -d' ' -f4 /proc/loadavg | cut -d/ -f1)" -le 2 ] && return 0
    sleep 1; i=$((i + 1))
  done
}

one() { # phase sim model len w n log
  local ph=$1 s=$2 m=$3 len=$4 w=$5 n=$6 log=$7
  wait_brief
  echo "$(date +%T) START $ph $s $m $len $(loadinfo)" >> "$LOADLOG"
  echo "$(date +%T) $ph $s $m $len (w=$w n=$n) $(loadinfo)" >&2
  run_logged "$log" "taskset -c $CORE $(cmd "$s" "$m" "$len" "$w" "$n")"
  echo "# END_LOAD $(cut -d' ' -f1-3 /proc/loadavg)" >> "$log"
  echo "$(date +%T) END   $ph $s $m $len $(loadinfo)" >> "$LOADLOG"
}

phase_run() {
  echo "$(date '+%F %T') session start: core $CORE, rounds $ROUNDS, JOPTS=$JOPTS, $(nproc) cores, $(loadinfo)" >> "$LOADLOG"
  for r in $(seq 1 "$ROUNDS"); do
    echo "$(date +%T) ROUND $r start $(loadinfo)" >> "$LOADLOG"
    for m in $MODELS; do
      for len in short long; do
        for s in $SIMS; do
          local w n
          case "$s:$len" in
            simoxide:short) w=50; n=200 ;; simoxide:long) w=3; n=20 ;;
            *:short) w=20; n=10 ;; *:long) w=1; n=3 ;;
          esac
          local log="$OUT/raw/warm/${s}__${m}__${len}.log"
          # resume: round r is done when the log has r successful processes
          [ -f "$log" ] && [ "$(grep -ac '^TIME .*rc=0' "$log")" -ge "$r" ] && continue
          one warm "$s" "$m" "$len" "$w" "$n" "$log"
        done
      done
      for s in $SIMS; do
        local cs=$s; [ "$s" = simoxide ] && cs=simoxide-cli
        local log="$OUT/raw/cold/${cs}__${m}__short__${r}.log"
        [ -f "$log" ] && grep -aq '^TIME .*rc=0' "$log" && continue
        rm -f "$log" "$log.err"
        one cold "$cs" "$m" short 0 1 "$log"
      done
    done
    echo "$(date +%T) ROUND $r end $(loadinfo)" >> "$LOADLOG"
  done
}

phase_report() {
  CMP_COLUMNS=present python3 "$C/summarize.py" "$OUT" > /dev/null
  python3 -I - "$OUT" <<'EOF' | tee "$OUT/rough.md"
import glob, os, re, statistics, sys
OUT = sys.argv[1]
KV = re.compile(r'(\w+)=(\S+)')
MODELS = ['x_ss_minimal', 'x_espresso', 'x_ss_mediastore', 'x_sl_mediastore', 'h13_passive_contention', 'x_pem_fork']
SIMS = ['simoxide', 'simulizar', 'simulizar-ssj']
EXPECT = {'simulizar': 'DesmoJSimEngineFactory', 'simulizar-ssj': 'SSJSimEngineFactory'}
warm, engines, procs = {}, {}, {}
for path in glob.glob(os.path.join(OUT, 'raw', 'warm', '*.log')):
    sim, model, length = os.path.basename(path)[:-4].split('__')
    rounds, cur, eng = [], None, []
    for line in open(path, 'rb'):
        line = line.decode('utf-8', 'replace')
        if line.startswith('# CMD'):
            cur = []; rounds.append(cur)
        elif line.startswith('# ENGINE'):
            eng.append(line.split()[2].rsplit('.', 1)[-1])
        elif line.startswith('RESULT ') and cur is not None:
            d = dict(KV.findall(line))
            if d.get('phase') == 'run':
                cur.append(d)
    warm[sim, model, length] = rounds
    engines[sim, model, length] = eng
cold = {}
for path in glob.glob(os.path.join(OUT, 'raw', 'cold', '*.log')):
    sim, model, length, rep = os.path.basename(path)[:-4].split('__')
    sim = sim.replace('-cli', '')
    for line in open(path, 'rb'):
        line = line.decode('utf-8', 'replace')
        if line.startswith('TIME ') and 'rc=0' in line:
            cold.setdefault((sim, model), []).append(float(dict(KV.findall(line))['wall_s']))
        if line.startswith('# ENGINE'):
            engines.setdefault((sim, model, 'cold'), []).append(line.split()[2].rsplit('.', 1)[-1])

def med(v): return statistics.median(v) if v else float('nan')
def fmt_ms(x):
    return '–' if x != x else (f'{x:.3f}' if x < 1 else f'{x:.2f}' if x < 10 else f'{x:.0f}' if x < 1e4 else f'{x:,.0f}')
def ratio(a, b):
    return '–' if a != a or b != b or b == 0 else (f'{a / b:,.0f}×' if a / b >= 100 else f'{a / b:.1f}×' if a / b >= 10 else f'{a / b:.2f}×')

problems = []
print('| model | len | SimOxide ms | SimuLizar DESMO-J ms | SimuLizar SSJ ms | SimOxide vs DESMO-J | SimOxide vs SSJ | SSJ vs DESMO-J | per round | results |')
print('|---|---|---:|---:|---:|---:|---:|---:|---|---|')
ssj_vs = []
for m in MODELS:
    for length in ('short', 'long'):
        if not any((s, m, length) in warm for s in SIMS):
            continue
        walls, res = {}, {}
        for s in SIMS:
            runs = [r for rnd in warm.get((s, m, length), []) for r in rnd if 'error' not in r]
            walls[s] = med([float(r['wall_ms']) for r in runs])
            res[s] = {(r.get('requests'), r.get('mean_rt')) for r in runs}
            nerr = sum(1 for rnd in warm.get((s, m, length), []) for r in rnd if 'error' in r)
            if nerr: problems.append(f'{s} {m} {length}: {nerr} failed runs')
            if s in EXPECT:
                e = engines.get((s, m, length), [])
                if not e or any(x != EXPECT[s] for x in e):
                    problems.append(f'{s} {m} {length}: engine {e}')
        # agreement: every run of every simulator should give the same requests and mean RT (same seed)
        allres = set().union(*res.values())
        if len(allres) == 1:
            req, rt = next(iter(allres)); agree = f'identical ({req} req, mean RT {float(rt):.6g})'
        else:
            rts = {s: sorted({float(x[1]) for x in v}) for s, v in res.items()}
            reqs = {s: sorted({x[0] for x in v}) for s, v in res.items()}
            flat = [x for v in rts.values() for x in v]
            spread = (max(flat) - min(flat)) / abs(statistics.mean(flat)) if flat and statistics.mean(flat) else float('nan')
            agree = ('DIFFER: ' if spread > 0.01 else 'close: ') + '; '.join(
                f"{s} req {'/'.join(reqs[s])} RT {'/'.join(f'{x:.6g}' for x in rts[s])}" for s in SIMS if s in rts)
            if spread > 0.01: problems.append(f'{m} {length}: mean RT spread {spread:.1%}')
        sx, d, j = walls['simoxide'], walls['simulizar'], walls['simulizar-ssj']
        rd = [med([float(r['wall_ms']) for r in rnd]) for rnd in warm.get(('simulizar', m, length), [])]
        rj = [med([float(r['wall_ms']) for r in rnd]) for rnd in warm.get(('simulizar-ssj', m, length), [])]
        per = ' / '.join(f'{a / b:.2f}' for a, b in zip(rd, rj))
        if d == d and j == j: ssj_vs.append(d / j)
        print(f'| `{m}` | {length} | {fmt_ms(sx)} | {fmt_ms(d)} | {fmt_ms(j)} | {ratio(d, sx)} | {ratio(j, sx)} | {ratio(d, j)} | {per} | {agree} |')
if ssj_vs:
    g = statistics.geometric_mean(ssj_vs)
    print(f'\nSSJ vs DESMO-J (warm, geometric mean over {len(ssj_vs)} configurations): {g:.2f}× '
          f'(range {min(ssj_vs):.2f}× to {max(ssj_vs):.2f}×)')

print('\nCold one-shot, short (process start to results, median of the rounds, s):\n')
print('| model | SimOxide CLI | SimuLizar DESMO-J | SimuLizar SSJ | SSJ vs DESMO-J |')
print('|---|---:|---:|---:|---:|')
for m in MODELS:
    if not any((s, m) in cold for s in SIMS):
        continue
    c = {s: med(cold.get((s, m), [])) for s in SIMS}
    print(f"| `{m}` | {c['simoxide']:.3f} | {c['simulizar']:.2f} | {c['simulizar-ssj']:.2f} | {ratio(c['simulizar'], c['simulizar-ssj'])} |")
    for s in EXPECT:
        e = engines.get((s, m, 'cold'), [])
        if not e or any(x != EXPECT[s] for x in e):
            problems.append(f'cold {s} {m}: engine {e}')

print('\nDrift check, warm median per round (ms): ' + '; '.join(
    f"{s} {m} {length}: " + '/'.join(fmt_ms(med([float(r['wall_ms']) for r in rnd])) for rnd in warm[s, m, length])
    for (s, m, length) in sorted(warm)))
print('\nEngine evidence (# ENGINE lines): ' + ', '.join(
    f'{s}: {sorted(set(x for k, v in engines.items() if k[0] == s for x in v))}' for s in EXPECT))
print('\nProblems: ' + ('; '.join(problems) if problems else 'none'))
EOF
}

for p in $PHASES; do "phase_$p"; done
