# Reference material

Official sources for the QVTo engine. The bulk is not committed (see `.gitignore`). This file
says where each part comes from and how to get it again.

Total size on disk: about 60 MB.

## Specifications (`specs/`, not committed)

| File | Source | Version | Size |
|---|---|---|---|
| `QVT-1.3.pdf` | <https://www.omg.org/spec/QVT/1.3/PDF> (formal/16-06-03) | MOF QVT 1.3, June 2016 (latest) | 1.6 MB |
| `OCL-2.4.pdf` | <https://www.omg.org/spec/OCL/2.4/PDF> (formal/14-02-03) | OCL 2.4, February 2014 (latest formal) | 1.9 MB |
| `QVT-1.3.txt`, `OCL-2.4.txt` | text extracted with `pypdf`; `=== page N ===` marks PDF pages | | 0.6 MB each |
| `qvt-1.3-xmi/*.xmi` | <https://www.omg.org/spec/QVT/20151201/> (normative machine-readable metamodels: EssentialOCL, ImperativeOCL, QVTOperational, QVTBase) | QVT 1.3 | 130 KB |

SHA-256: `QVT-1.3.pdf` 5d795f64…c9db, `OCL-2.4.pdf` b97e46a9…7a7ef.

Licence: OMG specifications are copyrighted by OMG and its submitters. They may be used to
implement the specification; we do not redistribute them.

Text extraction (any Python with `pypdf`):

```sh
python3 -m venv /tmp/v && /tmp/v/bin/pip install pypdf
/tmp/v/bin/python -I -c 'import sys; from pypdf import PdfReader
r = PdfReader(sys.argv[1]); open(sys.argv[2], "w").write("".join(f"\n\f=== page {i+1} ===\n" + (p.extract_text() or "") for i, p in enumerate(r.pages)))' QVT-1.3.pdf QVT-1.3.txt
```

### Relevant chapters

Printed page numbers. The PDF page is about the printed page plus 18 (QVT) or 16 (OCL).

QVT 1.3:

| Section | Pages | Contents |
|---|---|---|
| 6.2.2, 6.5 | 10, 12 | blackbox implementations; OCL usage in QVT |
| 8.1 | 65–87 | Operational Mappings overview: model types, extents, libraries, mappings, object creation, constructors, helpers, intermediate data, tracing and resolve (8.1.11), late resolve, updating objects, composition, disjuncts and overloading (8.1.14), inherits/merges (8.1.15), type extensions, imperative expressions, null (8.1.19), invalid (8.1.20) |
| 8.2.1 | 88–117 | QVTOperational abstract syntax and semantics (OperationalTransformation, Module, ModelParameter, MappingOperation, MappingBody, ResolveExp, ObjectExp, …) |
| 8.2.2 | 117–138 | ImperativeOCL: BlockExp, ComputeExp, WhileExp, ForExp, ImperativeIterateExp (xcollect, xselect, …), SwitchExp, VariableInitExp, AssignExp, TryExp, CatchExp, RaiseExp, ReturnExp, BreakExp, ContinueExp, LogExp, AssertExp, InstantiationExp, Dict/List types, … |
| 8.3 | 138–161 | QVTo standard library: predefined types, synonyms, operations on objects/elements/models/lists/dicts/strings (8.3.16, 35 operations), numeric types, `allInstances` |
| 8.4 | 161–172 | Concrete syntax: files, comments, strings, shorthands, metamodel notation, **EBNF (8.4.7)**, deprecated syntax, scoped identifiers |

OCL 2.4:

| Section | Pages | Contents |
|---|---|---|
| 7 | 5–36 | the language (basic values, objects and properties, collection operations, resolving properties) |
| 9 | 69–108 | concrete syntax (grammar with disambiguating rules) |
| 10 | 109–150 | semantics (values package, evaluation) |
| 11 | 151–184 | **standard library**: OclAny/OclVoid/OclInvalid (11.2–11.3), primitive types and their operations (11.4–11.5), collection types and operations (11.6–11.7), iterators and their mapping rules (11.8–11.9) |
| A | 203–239 | formal semantics (annex A) |

## Eclipse QVTo (`eclipse-qvto/`, not committed)

- Source: <https://github.com/eclipse-qvto/org.eclipse.qvto>, branch `master`, commit
  `8c01c4f6ff44e9e29fb667b2744724d39c4377dc` (2026-09-15). Bundle versions 3.11.2.qualifier.
  Latest release tag at download time: `3.11.4`.
- Licence: EPL-2.0.
- Fetched as a blobless shallow clone with a sparse checkout (32 MB):

```sh
git clone --depth 1 --filter=blob:none --no-checkout https://github.com/eclipse-qvto/org.eclipse.qvto.git eclipse-qvto
cd eclipse-qvto && git sparse-checkout init --no-cone
printf '%s\n' /plugins/org.eclipse.m2m.qvt.oml/ /plugins/org.eclipse.m2m.qvt.oml.cst.parser/ \
  /plugins/org.eclipse.m2m.qvt.oml.ecore.imperativeocl/ /plugins/org.eclipse.m2m.qvt.oml.runtime/ \
  /plugins/org.eclipse.m2m.qvt.oml.common/ /plugins/org.eclipse.m2m.qvt.oml.emf.util/ \
  /plugins/org.eclipse.m2m.qvt.oml.ocl/ /plugins/org.eclipse.m2m.qvt.oml.project/ \
  /plugins/org.eclipse.m2m.qvt.oml.samples/ /tests/ '!/tests/org.eclipse.m2m.tests.qvt.oml.ui/' \
  /doc/ '/LICENSE*' '/NOTICE*' '/*.md' > .git/info/sparse-checkout
git checkout
```

Key parts:

| Path | Contents |
|---|---|
| `plugins/org.eclipse.m2m.qvt.oml.cst.parser/cst/*.gi` | LPG grammars: `QVTOParser.gi` (2 384 lines), `ImperativeOCL.gi` (1 983), `EssentialOCL.gi` (1 484), lexers and keyword lexers. The authoritative concrete syntax. |
| `plugins/org.eclipse.m2m.qvt.oml/src` | compiler (`internal/qvt/oml/compiler`), AST, evaluator (`internal/qvt/oml/evaluator`, `QvtOperationalEvaluationVisitorImpl`), standard library (`internal/qvt/oml/stdlib`), trace, standalone API (`TransformationExecutor`, `BasicModelExtent`, `ExecutionContextImpl`) |
| `plugins/org.eclipse.m2m.qvt.oml.ecore.imperativeocl` | ImperativeOCL metamodel (generated EMF code) |
| `tests/org.eclipse.m2m.tests.qvt.oml` | the test suite: `parserTestData/models/*` (transformations with `in.*` and `expected.*`), `parserTestData/sources` (compiler tests), `deployed`, `models` (test metamodels), Java blackboxes in `src` |
| `plugins/org.eclipse.m2m.qvt.oml.samples` | samples incl. SimpleUML→RDB |

Notes on the semantics and quirks: `notes/eclipse-semantics.md`. Test inventory:
`inventory/eclipse-tests.tsv` and `inventory/README.md`.

## Classic Eclipse OCL (`eclipse-ocl/`, not committed)

- Source: <https://github.com/eclipse-ocl/org.eclipse.ocl>, branch `master`, commit
  `93585c74f385d589e9dc91ad2d9c92da1ef632f6` (2026-09-15). Licence: EPL-2.0.
- Sparse checkout (14 MB) of `plugins/org.eclipse.ocl`, `plugins/org.eclipse.ocl.ecore`,
  `plugins/org.eclipse.ocl.common`, `tests/org.eclipse.ocl.ecore.tests`, `tests/org.eclipse.ocl.tests`.
- QVTo is built on this "classic" OCL (`org.eclipse.ocl`, `org.eclipse.ocl.ecore`), not on the
  newer Pivot OCL. Its grammar (`org/eclipse/ocl/parser/*.gi`), evaluation visitor
  (`EvaluationVisitorImpl`) and collection utilities (`util/CollectionUtil`) define the OCL part
  of QVTo's behaviour.

## Java reference runner (`runner/`)

A headless Eclipse QVTo, run on a flat classpath without OSGi. Source `runner/src/qvtorun/QvtoRun.java`,
built with `javac` only.

- Jars (`runner/lib/qvto-3.11.0/`, 25 jars, 9.7 MB, not committed): copied from the local
  Slingshot install `old/palladio-research/work-slingshot/sl-install/plugins` (Eclipse 2025-12
  train): QVTo 3.11.0.v20251124-1059, classic OCL 3.22.0.v20240902-1518, EMF Ecore 2.41.0,
  EMF Common 2.44.0, XMI 2.39.0, LPG runtime 2.0.17, Equinox common/registry/preferences,
  core.runtime/jobs/contenttype/resources, OSGi 3.24, `org.osgi.service.prefs`.
- Other sources for the jars: QVTo is not on Maven Central. Releases up to 3.10.8 are p2
  repositories at `https://download.eclipse.org/mmt/qvto/updates/releases/<version>/`; 3.11.x
  ships with the Eclipse simultaneous release (`https://download.eclipse.org/releases/2025-12/`).
  Palladio 5.2.2 (SimuLizar's product) bundles QVTo 3.10.7
  (`old/palladio-research/work-simucom/palladio-5.2.2/plugins`); copy those jars into
  `lib/qvto-3.10.7` and build with `LIB=lib/qvto-3.10.7` to get SimuLizar's exact version.

```sh
reference/runner/build.sh       # javac; LIB=lib/<set> selects the jar set
reference/runner/qvto-run --qvto T.qvto [--mm a.ecore,b.ecore] \
    --param in:in.xmi --param out:out.xmi [--param inout:m.xmi] \
    [--config key=value] [--blackbox fq.Class] [--log log.txt] [--repeat N] [--expect expected.xmi]
reference/runner/tools/smoke-parserTestData.sh OUTDIR   # all simple parserTestData models
```

The runner registers every package of the `--mm` files by nsURI, runs `TransformationExecutor`
with one `ModelExtent` per `--param` (in signature order), prints the load and execution
diagnostics, the log and the per-run time, and saves `out` and `inout` extents as XMI.
`--expect` checks the first output extent with `EcoreUtil.equals` (order-sensitive). It runs
with `nice -n 19` and `-Xmx2g`.

Smoke result (2026-10-16): of the 174 test folders with exactly one `.qvto`, one `in.*` and one
`expected.*`, 136 produce an output equal to `expected.*`. 25 fail to compile because the
harness does not yet provide what the Eclipse test bundle registers (metamodels from
`plugin.xml`, Java blackboxes, imports resolved from the workspace). 13 differ, mostly because
they need configuration properties or the suite's own comparer. A cold JVM needs about 0.4 s to
compile a transformation and about 30 ms to run a small one.
