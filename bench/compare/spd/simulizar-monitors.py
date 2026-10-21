#!/usr/bin/env python3
"""Rewrites monitor repositories in place for SimuLizar-flavour SPD runs: SimuLizar needs a
time-driven processing type on utilisation monitors (Slingshot's examples use FeedThrough).
The window (10 s, every 10 s) is the one of crates/simoxide-reconf/tests/hpa.rs.

usage: simulizar-monitors.py FILE.monitorrepository ..."""
import sys

UTIL = "_QIb6cikUEeSuf8LV7cHLgA"  # commonMetrics.metricspec: utilisation of an active resource
FEED = 'xsi:type="monitorrepository:FeedThrough"'
TIMED = 'xsi:type="monitorrepository:TimeDriven" windowLength="10.0" windowIncrement="10.0"'

for p in sys.argv[1:]:
    out, util, n = [], False, 0
    for line in open(p, encoding="utf-8").read().split("\n"):
        if UTIL in line:
            util = True
        elif util and "processingType" in line:
            if FEED in line:
                line = line.replace(FEED, TIMED)
                n += 1
            util = False
        out.append(line)
    open(p, "w", encoding="utf-8").write("\n".join(out))
    print(f"{p}: {n} utilisation monitor(s) time-driven")
