# Source patches (det mode)

One directory per bundle symbolic name. `orig/` is the upstream source of each replaced class, `src/`
the patched copy (edits marked `SimOxide reference patch S0-Pn`). `setup.sh` compiles both, checks that
`orig/` has the member signatures of the installed class (`javap -p`, lambda names ignored) and writes
a copy of the installed jar with the patched classes to `build/patched/`; only the `det` Equinox
configuration uses it.

| Patch | Bundle / class | Upstream |
|---|---|---|
| S0-P2 | `org.palladiosimulator.analyzer.slingshot.behavior.resourcesimulation` / `ProcessorSharingResource`: job table `Hashtable` → `LinkedHashMap` | Palladio-Analyzer-Slingshot commit `8bf9adc` (2026-08-21; the nightly 202609010045 was built from it), Apache License 2.0 (bundle manifest) |

(S0-P1, `Result`'s `HashSet` → `LinkedHashSet`, is covered by the seed agent's ordering rule and has no
source patch.) Documentation: `docs/reference-simulator/slingshot.md`.
