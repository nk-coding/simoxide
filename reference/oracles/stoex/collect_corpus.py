#!/usr/bin/env python3
"""Collect every StoEx specification string from the PCM models under the given roots.

Usage: collect_corpus.py ROOT... > corpus_models.txt
Writes one expression per line, JSON-encoded (so newlines/quotes survive), sorted, unique.
"""
import json, os, sys
import xml.etree.ElementTree as ET

EXTS = ('.repository', '.usagemodel', '.resourceenvironment', '.system', '.allocation')
found = set()
for root in sys.argv[1:]:
    for d, _, files in os.walk(root):
        for f in files:
            if not f.endswith(EXTS):
                continue
            try:
                tree = ET.parse(os.path.join(d, f))
            except Exception:
                continue
            for el in tree.iter():
                s = el.attrib.get('specification')
                if s is not None:
                    found.add(s)
for s in sorted(found):
    print(json.dumps(s))
