# Instructions for AI coding agents

## Expose new features through the server

When you add a new feature, also make it available through the HTTP and MCP server
(`crates/simoxide-server`), in the same change, unless you are explicitly told otherwise. This
covers anything a user can configure, run or read, such as a new run option, stop condition,
model format, metric or result. Bug fixes and internal refactorings need nothing here.

Both protocols share one set of operations, so add the feature once and expose it in each:

| Where | What |
|---|---|
| `src/request.rs` | new request fields (`RunOptions`, `Condition`, ...) |
| `src/runner.rs` | apply them to the run; reject bad values with an error |
| `src/api.rs` | new operations on `Service`, shared by both protocols |
| `src/http.rs` | new REST routes, and the route table in the module documentation |
| `src/mcp.rs` | new tools and arguments in `tools()`, so that agents can discover them |
| `tests/server.rs` | an end-to-end test through HTTP and MCP |
| `docs/guide/server.md` | the user documentation |

If a feature cannot be exposed in a sensible way, ask instead of skipping it.
