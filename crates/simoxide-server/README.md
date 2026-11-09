# simoxide-server

SimOxide as a service: an HTTP server (REST and server-sent events) and an MCP server (stdio and
streamable HTTP) that take Palladio models, simulate them in the background, publish live
progress and logs, stop runs by a confidence interval, a timeout or a count, and return the
measurements.

```sh
cargo build --release -p simoxide-server
./target/release/simoxide-server --http 127.0.0.1:8080 --allow-dir corpus
./target/release/simoxide-server --mcp-stdio --allow-dir corpus
```

User documentation: [`docs/guide/server.md`](../../docs/guide/server.md).

| Module | Contents |
|---|---|
| `request` | the request: model files or directory, run configuration, termination tree, progress options |
| `job` | jobs, their shared state, log ring buffer and long polling; the `JobManager` (slots, retention, caps) |
| `runner` | model loading, engine choice (process engine, SPD, Slingshot-exact), the driver loop |
| `monitor` | the pause decision before each event, incremental statistics, termination evaluation |
| `stats` | Student t quantiles, batch means, running series statistics |
| `api` | the operations shared by both protocols (`Service`) |
| `http` | axum router: REST, SSE, MCP over streamable HTTP, token and origin checks |
| `mcp` | MCP JSON-RPC handler, tool definitions, stdio transport |

Tests: `cargo test -p simoxide-server` (unit tests, and `tests/server.rs`: end-to-end runs of
corpus models through the router, MCP over HTTP, SSE, SPD, Slingshot-exact).
