#!/usr/bin/env python3
"""Handler exceptions of a reference run (SS-BUS-7): one line per "An error occurred" of the bus,
`<exception>: <message> | <first Slingshot frame>`, in order. Identity hash codes in the messages
(`Class@1b2c3d`, from `Object.toString`) are replaced by `@*`: they differ between JVMs and change
nothing else.

    tools/errors.py RUN_LOG > errors.txt
"""
import re
import sys


def main():
    lines = open(sys.argv[1], errors="replace").read().splitlines()
    for i, line in enumerate(lines):
        m = re.search(r"An error occurred: (\S+):: (.*)$", line)
        if not m:
            continue
        frame = next((x.strip()[3:] for x in lines[i + 1:i + 40]
                      if "at org.palladiosimulator.analyzer.slingshot." in x), "?")
        msg = re.sub(r"@[0-9a-f]{4,8}\b", "@*", m.group(2))
        print(f"{m.group(1)}: {msg} | {frame}")


if __name__ == "__main__":
    main()
