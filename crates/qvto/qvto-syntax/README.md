# qvto-syntax

Lexer, parser, AST, diagnostics and unparser of QVT Operational Mappings with ImperativeOCL and
the OCL it embeds, as Eclipse QVTo 3.11 accepts it. No dependencies.

```rust
let parsed = qvto_syntax::parse(src);          // never fails; recovers from errors
for d in &parsed.diagnostics { /* severity, code, span, Eclipse-style message */ }
let unit: &qvto_syntax::ast::Unit = &parsed.unit;
let text = qvto_syntax::print::unit(unit);     // unparse
let dump = qvto_syntax::dump::unit(unit);      // span-free S-expression, for comparisons
```

## Layout

| File | Contents |
|---|---|
| `src/generated.rs` | **generated** from the LPG grammars: `TokenKind`, keyword table, punctuation table, ASCII character classes, keyword groups, binary operator levels |
| `src/lexer.rs` | lexer: tokens with byte spans, comments, LPG lexer quirks |
| `src/escape.rs` | string and quoted-name decoding (Eclipse's escape rules, including its octal bug) |
| `src/parser/` | recursive descent with precedence climbing: `module.rs` (units, modules, elements), `expr.rs` (expressions, statements); every function cites the grammar rules it implements |
| `src/ast.rs` | the AST; every node has a `Span` |
| `src/diag.rs` | `Diagnostic` with `Severity`, a stable `Code`, span and message |
| `src/span.rs` | spans, line index (LF, CR LF and lone CR), UTF-16 offset conversion |
| `src/print.rs`, `src/dump.rs` | unparser and span-free dump |
| `gen/lpg.rs` | the table generator (a `.gi` reader) |
| `tools/oracle/` | Eclipse's parser as a parse-only oracle (Java, against the reference runner's jars) and the scripts that make the test fixtures |

## Generated tables

`src/generated.rs` is derived from Eclipse's grammar files, not written by hand:

| Table | Source |
|---|---|
| keywords (119) | `%Export` of `QVTOKWLexer.gi` → `ImperativeOCLKWLexer.gi` → `EssentialOCLKWLexer.gi`; checked against each keyword's spelling rule (`s e l f`) and against the parsers' `%KeyWords` |
| other terminals (42) | `%Export` of `QVTOLexer.gi` → `ImperativeOCLLexer.gi` → `EssentialOCLLexer.gi` |
| punctuation spellings (35), longest-match lexing | lexer rules `Token ::= '<' '=' /.makeToken($_LESS_EQUAL)./` with `%DropRules` applied; checked against the parsers' `%Terminals` aliases |
| ASCII character classes | the `tokenKind[]` table of `lpg/LexerBasicMapF.gi` and the lexer rules `Letter`, `Digit`, `WSChar`, `Identifier`; the end-of-input characters from its `getKind` |
| keyword groups (20) | single-token alternatives of parser nonterminals (`primitiveTypeCS`, `CollectionTypeIdentifierCS`, `feature_key`, `resolveOp`, `imperativeIteratorExpCSToken12`, …) |
| binary operator levels (8) and prefix operators | the `impliesNotNameNotLetCS` → … → `unaryNotNameNotLetCS` cascade of `EssentialOCL.gi`, with QVTo's `!=` |

The parser refers only to these tables (`groups::RESOLVE_OP`, `binary_level`, …); it has no keyword
or operator lists of its own. The AST stores keyword-determined choices as the keyword's
`TokenKind`.

Regenerate after changing the grammar sources (needs `reference/eclipse-qvto`, see
`reference/README.md`, or `QVTO_ECLIPSE_DIR`):

```sh
cargo run -p qvto-syntax --example gen_tables            # writes src/generated.rs
cargo run -p qvto-syntax --example gen_tables -- --check # fails if out of date
```

`tests/generated_tables.rs` fails when the committed file differs from what the generator derives
(skipped when the reference checkout is absent).

## How it follows Eclipse

- The authority is the LPG grammar (`QVTOParser.gi` → `ImperativeOCL.gi` → `EssentialOCL.gi`), not
  the QVT 1.3 EBNF. LPG builds a backtracking LALR(2) parser; where it backtracks, the
  hand-written parser decides explicitly: C-style `if (c) s` versus `if (c) then …` after the
  condition (a parenthesised condition not followed by `then` is the statement form, which is
  only allowed as a statement); `src->name(v | e)` iterators versus calls by trying the
  declarators up to `|`; the dangling `else`/`endif` bind to the innermost `if` (LPG shifts).
- Lexer quirks: U+00B4 and U+FFFF end the input; `<<`, `>>`, `...`, `!=`, `::=`, `+=` are single
  tokens; `1.` is INTEGER DOT and `1...` INTEGER DOTDOT DOT; `1e` is an invalid token; strings may
  contain raw tabs and line breaks; invalid characters and tokens are reported and dropped.
- Checks of Eclipse's parser actions: invalid escapes, two string literals on one line, integer
  literals beyond `long`, the `while (x := e; c)` header, deprecated switch syntax, `import`
  placement, elements before the module header, `helper main`, files without a module.
- Messages imitate LPG's (`"X" expected after "Y"`, `"…" unexpected token(s)`, `token "…" is
  invalid`, `End-of-File reached after this token …`) and use Eclipse's texts where they come
  from `messages.properties`. Recovery imitates LPG's single-token deletion and insertion repairs
  and otherwise resynchronises at `;`, `}` and element keywords.

## Verification

All against Eclipse QVTo 3.11's own parser, run parse-only by `tools/oracle/ParseOracle.java`
(lexer, LPG parser and parser actions; no semantic analysis). Fixtures are committed; the tests
skip the parts whose corpus is not on disk.

| Test | What | Result (2026-10-16) |
|---|---|---|
| `corpus::eclipse_agreement` | every `.qvto` of the Eclipse checkout (1 046 test files + 1 sample), SimuLizar (37) and the SPD transformations (18): same problems as Eclipse (severity, UTF-16 offset, length, line, message) | 1 102 / 1 102 files identical; 1 058 without problems, 11 with syntax errors, 33 with parser warnings only |
| `corpus::round_trip` | parse → print → parse gives no errors and the same dump; printing is idempotent | 1 091 / 1 091 error-free files |
| `parser_cases` | the 155 `TestQvtParser` cases | all 62 cases that expect no errors have no syntax errors; 10 of the 93 error cases have syntax errors (the others are semantic, for `qvto-sema`); in the 81 annotated cases every syntax diagnostic hits an annotated span exactly (26 of 351 annotations are syntax-level) |
| `mutants` | 3 306 deterministic token mutants of the corpus (delete, duplicate, swap, replace) | accept/reject identical on 3 306 / 3 306; of the 3 133 Eclipse rejects, first error at the same offset in 1 516 (48 %), same span and message in 672 (21 %) |
| `edge_cases` | 182 hand-written snippets around the grammar's corners | accept/reject identical on all |
| `parser_unit`, unit tests | precedence, `if` forms, iterators, literals, escapes, lexer quirks, nesting limit | |

Speed: the whole corpus (1 102 files, 1.52 MB) parses in 12.5 ms single-threaded in release
mode, 121 MB/s, about 11 µs per file (`examples/parse_files.rs --time 20`). Eclipse's parser on
the same files in one warm JVM: 139 ms, 10.9 MB/s.

Robustness: nesting deeper than 256 levels is reported and skipped; a level takes at most about
6 KB of stack in debug builds and 3 KB in release builds, so the parser fits a 2 MB thread stack.
The deepest nesting in the corpus is 14.

### Regenerating the fixtures

```sh
crates/qvto-syntax/tools/oracle/make-fixture.sh         # tests/fixtures/eclipse-parse-oracle.tsv
crates/qvto-syntax/tools/oracle/make-mutant-fixture.sh  # tests/fixtures/eclipse-mutants-oracle.tsv
find … -name '*.qvto' | crates/qvto-syntax/tools/oracle/run.sh [-time N]   # ad hoc
```

`run.sh` needs a JDK and the jars of `reference/runner/lib/qvto-3.11.0`. Eclipse's parser
itself throws a `ClassCastException` in an error-recovery action on one mutant; the oracle records
that as an error.

## Differences from Eclipse

Accepted and rejected inputs are meant to be identical; the tests above found no difference.
Known differences in what is reported:

| Point | Eclipse | qvto-syntax |
|---|---|---|
| Error recovery after the first error | LPG's repair search (insertion, deletion, substitution, merging, scope recovery) | single-token deletion/insertion like LPG, otherwise resynchronisation; later errors and the first error's span can differ (see `mutants`) |
| "A file must declare at least one transformation or library!" | reported with token indices as offsets (a bug in `AbstractQVTParser.setupTopLevel`) | the span of the unit's elements |
| Lexer error inside a string after `\\` followed by a line break | LPG's LALR(2) lexer ends the token early | the string continues to its closing quote |
| Control characters inside `/* */` comments | invalid token | accepted |
| Parse tree | CST objects (`org.eclipse.m2m.internal.qvt.oml.cst`) | own AST; adjacent string literals are joined into one `StringLit`, parentheses kept as `Paren` |

Not reproduced on purpose: the grammar's content-assist variants (`cst/completion`), the CST token
links, and `ProblemOption` settings of classic OCL (e.g. conceptual operation names `a.and(b)`
are accepted without the optional warning, as in QVTo's default).

## For semantic analysis (Q2)

- `Element` lists keep source order; the module checks (`import` placement, elements before the
  header, files with several modules) are already done here. Splitting top-level elements into
  modules (Eclipse's `setupTopLevel`) is left to sema.
- Names: `NameKind::Quoted` names are decoded; `NameKind::Keyword` marks `this`, `result`, `self`,
  `main` and restricted keywords after `::`.
- `ScopedName` keeps the context: a type (`Set(T)::op`) or leading path segments (`A::B::op`).
- `ExprKind::Nav` with `NavOp::Arrow` is the `->prop` shorthand (`xcollect`); `XSelect` is
  `[…]`/`![…]`; `Call` with a `CONCEPTUAL_OPERATION` keyword name is `a.and(b)` style.
- `If::c_style`, `VarExp::parens`, `Population::explicit`, `SwitchAlt::deprecated` keep surface
  forms sema may need for messages.
- Integer literals are `i64` (Java `long`); reals keep their text for Java-compatible parsing.
