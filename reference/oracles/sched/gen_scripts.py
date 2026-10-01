#!/usr/bin/env python3
"""Writes the scheduler oracle scripts (scripts/*.txt). Deterministic (fixed seeds)."""
import os, random

D = os.path.join(os.path.dirname(os.path.abspath(__file__)), "scripts")
os.makedirs(D, exist_ok=True)

def write(name, header, lines, comment=""):
    with open(os.path.join(D, name + ".txt"), "w") as f:
        if comment:
            f.write("# " + comment + "\n")
        f.write(header + "\n")
        for l in lines:
            f.write(l + "\n")

HAND = {
 "ps1_basic": ("resource ps 1 1.0", ["job a 0 1.0", "job b 0.5 0.3", "job c 0.6 2.0"], "single core PS, overlapping jobs"),
 "ps1_ties": ("resource ps 1 1.0", ["job a 0 1.0", "job b 0 1.0", "job c 0 1.0", "job d 1 0.5", "job e 1 0.5",
               "job f 5 0.25 now 0.25", "job g 5 0.25 now 0.25"], "equal demands, simultaneous arrivals and completions"),
 "ps2_ties": ("resource ps 2 1.0", ["job a 0 1.0", "job b 0 1.0", "job c 0 1.0", "job d 0 1.0", "job e 0 1.0",
               "job f 0.5 2.0", "job g 0.5 2.0"], "2 cores, 5 equal jobs, then 2 more"),
 "ps3_cores": ("resource ps 3 2.0", ["job a 0 1.0", "job b 0.1 2.0", "job c 0.2 3.0", "job d 0.3 4.0",
               "job e 0.4 5.0", "job f 0.5 6.0", "job g 0.6 7.0", "job h 4.0 0.5"], "3 cores, rate 2"),
 "ps4_cores": ("resource ps 4 1.0", ["job a 0 0.1", "job b 0 0.2", "job c 0 0.3", "job d 0 0.4",
               "job e 0 0.5", "job f 0 0.6", "job g 0.05 0.05", "job h 0.05 0.05", "job i 0.05 0.05"], "4 cores, n crosses cores"),
 "ps_gaps": ("resource ps 1 1.0", ["job a 0 1.0", "job b 0.000001 1.0", "job c 0.000005 1.0", "job d 0.0000099 0.5",
               "job e 0.00001 0.5", "job f 0.000011 0.5", "job g 3.5 0.00001", "job h 3.500001 0.000001"],
             "arrivals closer than MathTools epsilon (1e-5): lost service time"),
 "ps_tiny": ("resource ps 2 1.0", ["job a 0 1e-12", "job b 0 0.0", "job c 0 -1.0", "job d 0 1e-9", "job e 0 5e-10",
               "job f 0.5 1e6", "job g 0.5 1e-7", "job h 0.5 0.9999999999", "job i 0.7 1e-15 now 0 now 2e-9"],
             "zero/negative demands (skipped), sub-JIFFY, huge"),
 "ps_closed": ("resource ps 2 1.5", ["job a 0 0.3 0.1 0.2 now 0.4 0 0.1", "job b 0 0.3 0.1 0.2 now 0.4 0 0.1",
               "job c 0.05 0.7 0.2 0.7 0.2 0.7", "job d 0.05 0.01 0.01 0.01 0.01 0.01 0.01 0.01"],
              "closed-loop jobs: think times (events) and immediate follow-ups (now)"),
 "fcfs_basic": ("resource fcfs 1 1.0", ["job a 0 1.0", "job b 0.5 0.3", "job c 0.6 2.0", "job d 0.6 0.1",
               "job e 10 0.5 now 0.5 0 0.5"], "FCFS"),
 "fcfs_gaps": ("resource fcfs 1 1.0", ["job a 0 0.00002", "job b 0.000001 1.0", "job c 0.000005 1.0",
               "job d 0.0000099 0.5", "job e 0.00001 0.5", "job f 2.5 1e-12", "job g 2.5 0.0", "job h 2.5 3e-6"],
              "FCFS with arrivals closer than 1e-5 and tiny demands"),
 "fcfs_closed": ("resource fcfs 1 1000.0", ["job a 0 300 100 200 now 400", "job b 0 300 0.1 200 now 400",
               "job c 0.05 700 0.2 700"], "FCFS closed loop, rate 1000 (linking-resource-like)"),
 "delay_basic": ("resource delay 1 1.0", ["job a 0 1.0", "job b 0 1.0", "job c 0.5 0.3", "job d 0.5 0.0",
               "job e 0.5 1e-12", "job f 1 0.5 now 0.5 0 0.5", "job g 1 0.5 0.5 0.5"], "delay"),
 "passive_1": ("passive 1", ["pjob a 0 1 1.0", "pjob b 0 1 0.5", "pjob c 0.2 1 0.3 0.1 1 0.2", "pjob d 0.2 1 0.1"],
               "capacity 1, FIFO waiting"),
 "passive_3": ("passive 3", ["pjob a 0 2 1.0", "pjob b 0 2 0.5", "pjob c 0 1 0.3", "pjob d 0.1 1 0.1",
               "pjob e 1.0 3 0.5 now 1 0.1", "pjob f 1.0 1 0.5"], "capacity 3, multi-unit requests, head blocks"),
}

for name, (h, lines, c) in HAND.items():
    write(name, h, lines, c)

def rnd_open(name, kind, cores, rate, n, lam, mean, seed, grid=None):
    r = random.Random(seed)
    t = 0.0
    lines = []
    for i in range(n):
        t += r.expovariate(lam)
        d = r.expovariate(1.0 / mean)
        if grid:  # coarse values -> many exact ties
            t = round(t / grid) * grid
            d = max(grid, round(d / grid) * grid)
        lines.append("job j%03d %r %r" % (i, t, d))
    write(name, "resource %s %d %r" % (kind, cores, rate), lines,
          "random open workload seed=%d n=%d lambda=%r mean=%r grid=%r" % (seed, n, lam, mean, grid))

def rnd_closed(name, kind, cores, rate, users, steps, mean_d, mean_z, seed):
    r = random.Random(seed)
    lines = []
    for u in range(users):
        parts = ["job", "u%02d" % u, repr(r.random() * mean_z), repr(r.expovariate(1.0 / mean_d))]
        for _ in range(steps - 1):
            z = r.random()
            parts.append("now" if z < 0.3 else repr(r.expovariate(1.0 / mean_z)))
            parts.append(repr(r.expovariate(1.0 / mean_d)))
        lines.append(" ".join(parts))
    write(name, "resource %s %d %r" % (kind, cores, rate), lines,
          "random closed workload seed=%d users=%d steps=%d" % (seed, users, steps))

for c in (1, 2, 3, 4):
    rnd_open("rnd_ps%d_open" % c, "ps", c, 1.0, 400, 0.9 * c, 1.0, 100 + c)
    rnd_open("rnd_ps%d_grid" % c, "ps", c, 1.0, 200, 1.2 * c, 1.0, 200 + c, grid=0.125)
    rnd_closed("rnd_ps%d_closed" % c, "ps", c, 3.0, 12, 25, 0.5, 1.0, 300 + c)
rnd_open("rnd_fcfs_open", "fcfs", 1, 1.0, 400, 0.9, 1.0, 401)
rnd_open("rnd_fcfs_grid", "fcfs", 1, 1.0, 200, 1.2, 1.0, 402, grid=0.125)
rnd_closed("rnd_fcfs_closed", "fcfs", 1, 2.0, 8, 25, 0.5, 1.0, 403)
rnd_open("rnd_delay_open", "delay", 1, 1.0, 400, 3.0, 1.0, 501)
rnd_closed("rnd_delay_closed", "delay", 1, 1.0, 8, 25, 0.5, 1.0, 502)

def rnd_wild(name, kind, cores, n, seed):
    """Stress: gaps of 0 / around the 1e-5 s threshold / exponential, demands spanning
    sub-JIFFY to large, zero and negative demands, closed-loop steps with zero think times."""
    r = random.Random(seed)
    def gap():
        u = r.random()
        return 0.0 if u < 0.25 else (r.uniform(0.0, 2e-5) if u < 0.5 else r.expovariate(2.0))
    def dem():
        u = r.random()
        if u < 0.05: return 0.0
        if u < 0.07: return -0.5
        if u < 0.15: return r.uniform(1e-13, 2e-9)
        if u < 0.3: return r.uniform(1e-6, 5e-5)
        return r.expovariate(2.0)
    t = 0.0
    lines = []
    for i in range(n):
        t += gap()
        parts = ["job", "w%03d" % i, repr(t), repr(dem())]
        for _ in range(r.randint(0, 3)):
            u = r.random()
            parts.append("now" if u < 0.3 else ("0" if u < 0.5 else repr(gap())))
            parts.append(repr(dem()))
        lines.append(" ".join(parts))
    write(name, "resource %s %d %r" % (kind, cores, 1.0), lines, "random stress seed=%d n=%d" % (seed, n))

for c in (1, 2, 3, 4):
    rnd_wild("rnd_ps%d_wild" % c, "ps", c, 80, 700 + c)
rnd_wild("rnd_fcfs_wild", "fcfs", 1, 80, 711)
rnd_wild("rnd_delay_wild", "delay", 1, 80, 712)

def rnd_passive(name, cap, users, steps, seed):
    r = random.Random(seed)
    lines = []
    for u in range(users):
        parts = ["pjob", "p%02d" % u, repr(r.random()), str(r.randint(1, cap)), repr(r.expovariate(2.0))]
        for _ in range(steps - 1):
            parts.append("now" if r.random() < 0.3 else repr(r.expovariate(1.0)))
            parts += [str(r.randint(1, cap)), repr(r.expovariate(2.0))]
        lines.append(" ".join(parts))
    write(name, "passive %d" % cap, lines, "random passive seed=%d" % seed)

rnd_passive("rnd_passive_1", 1, 6, 20, 601)
rnd_passive("rnd_passive_4", 4, 10, 20, 602)
