# Source patches (det mode)

One directory per bundle symbolic name. `orig/` is the upstream source of each replaced class, `src/`
the patched copy (edits marked `SimOxide reference patch S0-Pn`). `setup.sh` compiles both, checks that
`orig/` has the member signatures of the installed class (`javap -p`, lambda names ignored) and writes
a copy of the installed jar with the patched classes to `build/patched/`; only the `det` Equinox
configuration uses it.

| Patch | Bundle / class | Upstream |
|---|---|---|
| S0-P2 | `org.palladiosimulator.analyzer.slingshot.behavior.resourcesimulation` / `ProcessorSharingResource`: job table `Hashtable` → `LinkedHashMap` | Palladio-Analyzer-Slingshot commit `8bf9adc` (2026-08-21; the nightly 202609010045 was built from it; the file is unchanged at `1bb601f`), EPL-2.0 (repository `LICENSE`, added in `4e2339c` on 2026-10-19) |

(S0-P1, `Result`'s `HashSet` → `LinkedHashSet`, is covered by the seed agent's ordering rule and has no
source patch.) Documentation: `docs/reference-simulator/slingshot.md`.

Licence: both copies are Slingshot code under the Eclipse Public License 2.0 (`LICENSES` section 1);
the modified `src/` copy stays EPL-2.0. The upstream file has no copyright header to keep.
