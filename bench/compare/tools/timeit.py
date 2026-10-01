#!/usr/bin/env python3
"""Runs one command (a shell-quoted string, executed without a shell) in its own process group and
appends its stdout to LOG (stderr to LOG.err) plus a line
  TIME wall_s=.. rss_kb=.. user_s=.. sys_s=.. rc=.. [timeout=1]
wall_s: fork to exit (perf_counter); rss_kb: GNU time %M (peak RSS of the largest process of the command).

  --sample CSV   every --interval s (default 2), append 'epoch_ms,t_s,rss_kb,threads,procs,mem_avail_kb' for the
                 whole process tree (sum of VmRSS and thread counts of all descendants)
  --timeout S    kill the process group after S seconds (rc=-9, 'timeout=1')
Always: kills the process group if MemAvailable drops below MEM_FLOOR_KB (env, default 8000000 = 8 GB;
'# KILLED' line in LOG).
usage: timeit.py [--sample CSV] [--interval S] [--timeout S] LOG 'command ...'"""
import os, shlex, signal, subprocess, sys, tempfile, time

args = sys.argv[1:]
opts = {}
while args and args[0].startswith('--'):
    opts[args[0][2:]] = args[1]
    args = args[2:]
log, cmd = args
interval = float(opts.get('interval', 2))
timeout = float(opts['timeout']) if 'timeout' in opts else None
sample = opts.get('sample')
mem_floor = int(os.environ.get('MEM_FLOOR_KB', 8000000))


def mem_avail():
    for line in open('/proc/meminfo'):
        if line.startswith('MemAvailable'):
            return int(line.split()[1])
    return 0


def tree(root):
    kids = {}
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                st = open(f'/proc/{p}/stat').read()
                ppid = int(st[st.rindex(')') + 2:].split()[1])
                kids.setdefault(ppid, []).append(int(p))
            except (OSError, ValueError):
                pass
    out, todo = [], [root]
    while todo:
        p = todo.pop()
        out.append(p)
        todo += kids.get(p, [])
    return out


def usage(pids):
    rss = thr = n = 0
    for p in pids:
        try:
            d = dict(l.split(':', 1) for l in open(f'/proc/{p}/status') if ':' in l)
            rss += int(d.get('VmRSS', '0 kB').split()[0])
            thr += int(d['Threads'])
            n += 1
        except (OSError, KeyError, ValueError):
            pass
    return rss, thr, n


fd, tmp = tempfile.mkstemp()
os.close(fd)
timed_out = killed = False
with open(log, 'ab') as out, open(log + '.err', 'ab') as err:
    t0 = time.perf_counter()
    proc = subprocess.Popen(['/usr/bin/time', '-f', '%M %U %S', '-o', tmp] + shlex.split(cmd), stdout=out,
                            stderr=err, start_new_session=True)
    sf = open(sample, 'a') if sample else None
    if sf and sf.tell() == 0:
        sf.write('epoch_ms,t_s,rss_kb,threads,procs,mem_avail_kb\n')
    import threading
    done = threading.Event()

    def watch():  # memory guard, timeout, sampling (the main thread only waits, for an exact wall time)
        global timed_out, killed
        next_sample = t0
        while not done.is_set():
            now = time.perf_counter()
            ma = mem_avail()
            if ma < mem_floor and not killed:
                killed = True
                os.killpg(proc.pid, signal.SIGKILL)
            if timeout is not None and now - t0 > timeout and not timed_out:
                timed_out = True
                os.killpg(proc.pid, signal.SIGKILL)
            if sf and now >= next_sample:
                rss, thr, n = usage(tree(proc.pid)[1:])  # without /usr/bin/time itself
                sf.write(f'{int(time.time() * 1000)},{now - t0:.2f},{rss},{thr},{n},{ma}\n')
                sf.flush()
                next_sample += interval
            done.wait(0.2)

    th = threading.Thread(target=watch, daemon=True)
    th.start()
    rc = proc.wait()
    wall = time.perf_counter() - t0
    done.set()
    th.join()
    if sf:
        sf.close()
try:
    rss, user, sys_ = open(tmp).read().split('\n')[-2].split()
except (ValueError, IndexError):
    rss = user = sys_ = '-1'
os.unlink(tmp)
with open(log, 'a') as f:
    if killed:
        f.write(f'# KILLED: MemAvailable < {mem_floor} kB\n')
    f.write(f"TIME wall_s={wall:.6f} rss_kb={rss} user_s={user} sys_s={sys_} rc={rc}"
            + (' timeout=1' if timed_out else '') + '\n')
