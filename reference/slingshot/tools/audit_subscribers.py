#!/usr/bin/env python3
"""Static audit of Slingshot's subscriber order (DESIGN-SPD.md X1; docs/spec/slingshot/core.md SS-BUS-2).

The bus registers a behaviour's @Subscribe methods in getDeclaredMethods() order, which the JVM does
not specify (it varied between JVMs in the S1 probe runs). The order of two subscribers of one class
can therefore only change a run if some event reaches both. This tool checks, on the Slingshot sources,
that no class has two @Subscribe methods that can receive the same event:

  * the parameter types overlap: a common subtype among the event classes of the sources (or one is a
    supertype of the other), and
  * the reified types overlap: one of them is not reified, or both share a reified type (PCM's
    reified element types, e.g. Start/Stop, StartAction/StopAction, are distinct concrete EClasses).

    tools/audit_subscribers.py [SLINGSHOT_BUNDLES]     exit 0: no overlap; prints the per-class table

Default sources: the pinned commit of DESIGN-SPD.md ([SS], 8bf9adc).
"""
import os
import re
import sys
from collections import defaultdict

DEFAULT = "/home/devbox/workspace/old/palladio-research/repos/Palladio-Analyzer-Slingshot/bundles"

DECL = re.compile(r"\b(class|interface|enum|record)\s+(\w+)(\s*<[^{]*?>)?\s*([^{;]*)\{", re.S)
SUB = re.compile(
    r"@Subscribe\s*(\((?P<args>[^)]*)\))?\s*(?:@\w+(?:\([^)]*\))?\s*)*"
    r"public\s+(?:final\s+)?[\w<>?,.\s\[\]]+?\s+(?P<name>\w+)\s*\(\s*(?:final\s+)?(?P<ptype>[\w.]+)",
    re.S)


def strip_generics(s):
    out, depth = [], 0
    for ch in s:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def parse(root):
    supers = defaultdict(set)   # simple name -> direct supertypes (simple names)
    abstract = set()
    subs = defaultdict(list)    # file:class -> [(method, param type, reified, priority)]
    for d, _, files in os.walk(root):
        if "/test" in d or "annotationprocessor" in d:
            continue
        for f in files:
            if not f.endswith(".java"):
                continue
            p = os.path.join(d, f)
            src = open(p, errors="replace").read()
            src = re.sub(r"/\*.*?\*/", "", src, flags=re.S)
            src = re.sub(r"//[^\n]*", "", src)
            for m in DECL.finditer(src):
                kind, name, _, rest = m.groups()
                rest = strip_generics(rest)
                pre = src[max(0, m.start() - 80):m.start()]
                if kind == "interface" or re.search(r"\babstract\s*$", pre.strip() + " ") or "abstract" in pre.split("\n")[-1]:
                    abstract.add(name)
                for kw in ("extends", "implements"):
                    mm = re.search(r"\b" + kw + r"\b(.*?)(\bimplements\b|\bpermits\b|$)", rest, re.S)
                    if mm:
                        for t in mm.group(1).split(","):
                            t = t.strip().split(".")[-1]
                            if t:
                                supers[name].add(t)
            cls = os.path.splitext(f)[0]
            for m in SUB.finditer(src):
                args = m.group("args") or ""
                reified = re.findall(r"(\w+)\.class", args)
                pr = re.search(r"priority\s*=\s*(-?\d+)", args)
                subs[os.path.relpath(p, root) + ":" + cls].append(
                    (m.group("name"), m.group("ptype").split(".")[-1], tuple(reified), int(pr.group(1)) if pr else 0))
    return supers, abstract, subs


def ancestors(supers, t, seen=None):
    seen = set() if seen is None else seen
    for s in supers.get(t, ()):
        if s not in seen:
            seen.add(s)
            ancestors(supers, s, seen)
    return seen


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else DEFAULT
    supers, abstract, subs = parse(root)
    types = set(supers) | {s for v in supers.values() for s in v}
    below = defaultdict(set)    # type -> itself and its subtypes among the parsed types
    for t in types:
        below[t].add(t)
        for a in ancestors(supers, t):
            below[a].add(t)
    problems, shared = [], []
    for cls, ms in sorted(subs.items()):
        for i in range(len(ms)):
            for j in range(i + 1, len(ms)):
                a, b = ms[i], ms[j]
                common = below.get(a[1], {a[1]}) & below.get(b[1], {b[1]})
                if not common:
                    continue
                if a[2] and b[2] and not set(a[2]) & set(b[2]):
                    shared.append((cls, a, b, "disjoint reified types"))
                    continue
                problems.append((cls, a, b, ", ".join(sorted(common))))
    n = sum(len(v) for v in subs.values())
    print(f"{len(subs)} classes with @Subscribe methods, {n} subscribers")
    for cls, a, b, why in shared:
        print(f"ok   {cls}: {a[0]}({a[1]}<{'|'.join(a[2])}>) / {b[0]}({b[1]}<{'|'.join(b[2])}>): {why}")
    for cls, a, b, common in problems:
        print(f"FAIL {cls}: {a[0]}({a[1]}) and {b[0]}({b[1]}) both receive {common}")
    prio = [(c, m) for c, ms in subs.items() for m in ms if m[3] != 0]
    for c, m in prio:
        print(f"note {c}: {m[0]} has priority {m[3]} (unused by the bus)")
    print("result:", "no class subscribes twice to one event type without disjoint reification"
          if not problems else f"{len(problems)} overlapping pairs")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
