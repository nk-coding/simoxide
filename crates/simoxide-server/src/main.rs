//! `simoxide-server`: the HTTP API and the MCP server of SimOxide (see `docs/guide/server.md`).

use simoxide_server::http::{DEFAULT_MAX_BODY_BYTES, HttpConfig};
use simoxide_server::job::{
    DEFAULT_LOG_CAPACITY, DEFAULT_MAX_JOBS, DEFAULT_MAX_RUN_SECS, DEFAULT_MAX_TUPLES, MIB,
};
use simoxide_server::mcp::Mcp;
use simoxide_server::{JobManager, ServerConfig, Service};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

/// Where HTTP is served without `--http` and `--mcp-stdio`.
const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:8080";
/// The environment variable that can hold the bearer token.
const TOKEN_ENV: &str = "SIMOXIDE_SERVER_TOKEN";
/// Exit code of bad command-line arguments.
const EXIT_USAGE: u8 = 2;

/// The help text, with the current defaults.
fn usage() -> String {
    format!(
        "usage: simoxide-server [--http ADDR] [--mcp-stdio] [options]

Serves SimOxide's Palladio simulator to other programs (HTTP: REST, server-sent events, MCP at
/mcp) and to AI agents (MCP over stdio). Without --http and --mcp-stdio it serves HTTP on
{DEFAULT_HTTP_ADDR}. Documentation: docs/guide/server.md.

  --http ADDR            serve HTTP on ADDR (e.g. 127.0.0.1:8080, 0.0.0.0:8080)
  --mcp-stdio            serve MCP on stdin/stdout (for MCP clients that launch the server)
  --allow-dir DIR        let requests name model directories below DIR (`model_dir`);
                         repeatable; default: none, models come with the request
  --max-concurrent N     simulations that run at once (default: the number of CPUs)
  --max-jobs N           simulations kept in memory (default {DEFAULT_MAX_JOBS}; oldest finished dropped)
  --max-run-secs S       wall-clock cap of one run (default {DEFAULT_MAX_RUN_SECS}, 0 = none)
  --max-tuples N         cap of recorded tuples per run, 16 bytes each (default
                         {DEFAULT_MAX_TUPLES}, 0 = none)
  --log-capacity N       log entries kept per simulation (default {DEFAULT_LOG_CAPACITY})
  --token TOKEN          require `Authorization: Bearer TOKEN` on HTTP (also the
                         environment variable {TOKEN_ENV})
  --max-body-mb N        largest HTTP request body (default {})
  --allow-any-origin     accept browser requests from any Origin (default: local only)
  -h, --help             this text",
        DEFAULT_MAX_BODY_BYTES / MIB
    )
}

/// The parsed command line.
struct Args {
    /// Address to serve HTTP on.
    http: Option<String>,
    /// Serve MCP on stdin and stdout.
    stdio: bool,
    server: ServerConfig,
    http_cfg: HttpConfig,
}

/// Parses the command line; the error (or the help text) goes to stderr.
fn parse() -> Result<Args, String> {
    let mut a = Args {
        http: None,
        stdio: false,
        server: ServerConfig::default(),
        http_cfg: HttpConfig {
            token: std::env::var(TOKEN_ENV).ok().filter(|t| !t.is_empty()),
            ..HttpConfig::default()
        },
    };
    let mut it = std::env::args().skip(1);
    while let Some(f) = it.next() {
        let mut val = || {
            it.next()
                .ok_or_else(|| format!("{f}: value missing\n{}", usage()))
        };
        let num = |s: String, f: &str| -> Result<u64, String> {
            s.parse::<u64>()
                .map_err(|_| format!("{f}: expected a non-negative integer, got {s}"))
        };
        match f.as_str() {
            "--http" => a.http = Some(val()?),
            "--mcp-stdio" => a.stdio = true,
            "--allow-dir" => {
                let d = PathBuf::from(val()?);
                if !d.is_dir() {
                    return Err(format!("--allow-dir {}: not a directory", d.display()));
                }
                a.server.allow_dirs.push(d);
            }
            "--max-concurrent" => a.server.max_concurrent = num(val()?, &f)?.max(1) as usize,
            "--max-jobs" => a.server.max_jobs = num(val()?, &f)?.max(1) as usize,
            "--max-run-secs" => {
                let s: f64 = val()?
                    .parse()
                    .map_err(|_| format!("{f}: expected seconds"))?;
                a.server.max_run_secs = (s > 0.0).then_some(s);
            }
            "--max-tuples" => {
                let n = num(val()?, &f)?;
                a.server.max_tuples = (n > 0).then_some(n);
            }
            "--log-capacity" => a.server.log_capacity = num(val()?, &f)? as usize,
            "--token" => a.http_cfg.token = Some(val()?),
            "--max-body-mb" => a.http_cfg.max_body_bytes = num(val()?, &f)? as usize * MIB,
            "--allow-any-origin" => a.http_cfg.allow_any_origin = true,
            "-h" | "--help" => return Err(usage()),
            o => return Err(format!("unknown flag {o}\n{}", usage())),
        }
    }
    if a.http.is_none() && !a.stdio {
        a.http = Some(DEFAULT_HTTP_ADDR.into());
    }
    Ok(a)
}

/// Serves HTTP on `addr` on a thread of its own, with its own async runtime.
fn spawn_http_server(
    svc: Arc<Service>,
    cfg: HttpConfig,
    addr: String,
) -> std::thread::JoinHandle<Result<(), String>> {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind(&addr)
                .await
                .map_err(|e| format!("--http {addr}: {e}"))?;
            let local = listener.local_addr().map(|a| a.to_string()).unwrap_or(addr);
            eprintln!(
                "simoxide-server: HTTP on http://{local} (REST /v1/simulations, MCP /mcp){}",
                if cfg.token.is_some() {
                    ", bearer token required"
                } else {
                    ""
                }
            );
            simoxide_server::http::serve(svc, cfg, listener)
                .await
                .map_err(|e| e.to_string())
        })
    })
}

fn main() -> ExitCode {
    let a = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let svc = Arc::new(Service {
        jobs: JobManager::new(a.server),
    });
    let http = a
        .http
        .map(|addr| spawn_http_server(svc.clone(), a.http_cfg.clone(), addr));
    if a.stdio {
        eprintln!("simoxide-server: MCP on stdio");
        if let Err(e) = simoxide_server::mcp::serve_stdio(Arc::new(Mcp { svc: svc.clone() })) {
            eprintln!("simoxide-server: stdio: {e}");
            return ExitCode::FAILURE;
        }
        // stdin closed: the client is gone; stop the runs unless HTTP keeps serving
        if http.is_none() {
            svc.jobs.cancel_all();
            return ExitCode::SUCCESS;
        }
    }
    if let Some(h) = http {
        match h.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                eprintln!("simoxide-server: {e}");
                return ExitCode::FAILURE;
            }
            Err(_) => return ExitCode::FAILURE,
        }
    }
    ExitCode::SUCCESS
}
