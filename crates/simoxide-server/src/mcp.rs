//! The Model Context Protocol (MCP) server: SimOxide's operations as MCP tools for AI agents.
//!
//! [`Mcp::handle`] answers one JSON-RPC 2.0 message (`initialize`, `ping`, `tools/list`,
//! `tools/call`; notifications get no answer). Two transports use it:
//!
//! * **stdio** ([`serve_stdio`]): newline-delimited JSON-RPC on stdin and stdout, as MCP
//!   clients launch local servers (`simoxide-server --mcp-stdio`). Requests are handled
//!   concurrently, so a client can call `get_status` while a `wait_for_simulation` waits.
//! * **streamable HTTP** (`POST /mcp` of [`crate::http`]): one JSON-RPC message or batch per
//!   request, answered with `application/json`. The server sends no server-initiated messages,
//!   so `GET /mcp` (the optional SSE stream) is not offered.
//!
//! Tool results carry the JSON both as text content and as `structuredContent`; failures are
//! tool results with `isError: true` (protocol errors only for malformed requests).
//!
//! Handling is asynchronous (tokio): `wait_for_simulation` waits without holding a thread, and
//! the other tools run on tokio's blocking threads.

use crate::api::{
    DEFAULT_MAX_ROWS_PER_SERIES, DEFAULT_RECENT_LOGS, MAX_LOG_PAGE, MAX_WAIT, ResultsQuery,
    Service, Until, to_json, without_series,
};
use crate::job::{ApiError, MIB, lock_ignoring_poison};
use crate::report::Level;
use crate::request::{
    DEFAULT_CONFIDENCE_LEVEL, DEFAULT_MIN_BATCHES, DEFAULT_MIN_SAMPLES,
    DEFAULT_PROGRESS_INTERVAL_MS, MAX_PROGRESS_INTERVAL_MS, MIN_PROGRESS_INTERVAL_MS,
    SeriesSelector, SimulationRequest,
};
use crate::stats::MIN_BATCHES_FOR_INTERVAL;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Protocol revisions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The JSON-RPC version of every message.
pub(crate) const JSONRPC_VERSION: &str = "2.0";
/// JSON-RPC error: the message is not valid JSON.
pub(crate) const RPC_PARSE_ERROR: i64 = -32700;
/// JSON-RPC error: the message is not a valid request.
const RPC_INVALID_REQUEST: i64 = -32600;
/// JSON-RPC error: no such method.
const RPC_METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC error: invalid parameters.
const RPC_INVALID_PARAMS: i64 = -32602;
/// Default of `wait_for_simulation`'s `timeout_secs`.
const DEFAULT_WAIT_SECS: f64 = 30.0;
/// Default of `get_logs`'s `limit`.
const DEFAULT_LOG_LIMIT: u64 = 200;
/// Default of `get_results`'s `max_csv_bytes`.
const DEFAULT_MAX_CSV_BYTES: usize = MIB;
/// Nesting levels of `all`/`any` the schema of a termination condition spells out.
const CONDITION_SCHEMA_DEPTH: u32 = 2;

/// What the server tells the agent in `initialize`.
pub const INSTRUCTIONS: &str = "SimOxide simulates Palladio Component Models (PCM) with SimuLizar 5.2.2's or Slingshot's semantics.

Workflow:
1. start_simulation with the model: either `files` (every model file as {name, content}: .repository, .system, .resourceenvironment, .allocation, .usagemodel, .measuringpoint, .monitorrepository, optional run.json, .spd, .hpa.json, .loadprofile.json) or `model_dir` (a directory on the server, see list_models). Set `termination` to stop by a confidence interval, a timeout and/or a count; the run needs at least one bounding stop (config.max_sim_time, config.max_measurements, termination.timeout_secs, termination.max_measurements, ...).
2. Follow it: wait_for_simulation (blocks until something changes or it ends) or get_status return the live progress: simulation time, finished usage-scenario runs, per-series mean/std/min/max and confidence intervals, and how far each termination criterion is. get_logs returns the log (model warnings, scaling-policy decisions, termination).
3. get_results once it has ended: per-series statistics with percentiles and confidence intervals, optionally the raw (time, value) rows (downsampled by default) or the measurements.csv.
cancel_simulation stops a run early and keeps its results so far.

Measurements: one series per (measuring point, metric) of the monitor repository, e.g. `Response Time Tuple` of a usage scenario or an operation, `Utilization of Active Resource Tuple` of a CPU. Times are in seconds of simulated time.";

/// The MCP request handler over a [`Service`].
pub struct Mcp {
    /// The operations the tools call.
    pub svc: Arc<Service>,
}

/// A JSON-RPC error response.
pub(crate) fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": JSONRPC_VERSION, "id": id, "error": {"code": code, "message": message.into()}})
}

/// A JSON-RPC success response.
fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": JSONRPC_VERSION, "id": id, "result": result})
}

impl Mcp {
    /// Answers a JSON-RPC message or batch (the messages of a batch one after the other);
    /// `None` if nothing in it needs an answer.
    pub async fn handle_message(&self, message: Value) -> Option<Value> {
        match message {
            Value::Array(batch) => {
                let mut replies = Vec::new();
                for m in batch {
                    replies.extend(self.handle(m).await);
                }
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            one => self.handle(one).await,
        }
    }

    /// Answers one JSON-RPC message; `None` for notifications and responses.
    pub async fn handle(&self, msg: Value) -> Option<Value> {
        let Value::Object(mut obj) = msg else {
            return Some(rpc_error(
                Value::Null,
                RPC_INVALID_REQUEST,
                "invalid request: not an object",
            ));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str).map(String::from) else {
            // a response from the client (we never send requests): ignore
            return id.map(|id| rpc_error(id, RPC_INVALID_REQUEST, "invalid request: no method"));
        };
        let id = id?; // notifications (`notifications/initialized`, `.../cancelled`) need no answer
        let mut params = obj.remove("params").unwrap_or(Value::Null);
        Some(match method.as_str() {
            "initialize" => {
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == asked)
                    .copied()
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                rpc_result(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {
                            "name": "simoxide",
                            "title": "SimOxide Palladio simulator",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "instructions": INSTRUCTIONS,
                    }),
                )
            }
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(id, json!({ "tools": tools() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // taken, not copied: the arguments of start_simulation carry the model
                let args = params
                    .get_mut("arguments")
                    .map_or_else(|| json!({}), Value::take);
                if !tools().iter().any(|t| t["name"] == name.as_str()) {
                    return Some(rpc_error(
                        id,
                        RPC_INVALID_PARAMS,
                        format!("unknown tool {name:?}"),
                    ));
                }
                rpc_result(id, self.call(&name, args).await)
            }
            "resources/list" => rpc_result(id, json!({"resources": []})),
            "resources/templates/list" => rpc_result(id, json!({"resourceTemplates": []})),
            "prompts/list" => rpc_result(id, json!({"prompts": []})),
            m => rpc_error(id, RPC_METHOD_NOT_FOUND, format!("method not found: {m}")),
        })
    }

    /// Runs a tool: the result as MCP `CallToolResult`.
    pub async fn call(&self, name: &str, args: Value) -> Value {
        match self.dispatch(name, args).await {
            Ok(v) => {
                let text = serde_json::to_string_pretty(&v).unwrap_or_default();
                // structuredContent must be an object
                let structured = if v.is_object() {
                    v
                } else {
                    json!({ "items": v })
                };
                json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": structured,
                    "isError": false,
                })
            }
            Err(e) => json!({
                "content": [{"type": "text", "text": format!("error: {}", e.message)}],
                "isError": true,
            }),
        }
    }

    /// Runs the tool `name` with the arguments `a`: `wait_for_simulation` waits here, every
    /// other tool runs on a blocking thread.
    async fn dispatch(&self, name: &str, a: Value) -> Result<Value, ApiError> {
        if name == "wait_for_simulation" {
            return wait_for_simulation(&self.svc, &a).await;
        }
        let (svc, name) = (self.svc.clone(), name.to_string());
        tokio::task::spawn_blocking(move || run_tool(&svc, &name, a))
            .await
            .map_err(|e| ApiError {
                status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                message: format!("internal error: {e}"),
            })?
    }
}

/// Runs a tool that does not wait.
fn run_tool(svc: &Service, name: &str, a: Value) -> Result<Value, ApiError> {
    match name {
        "server_info" => Ok(svc.info()),
        "list_models" => Ok(json!({ "models": svc.models()? })),
        "start_simulation" => start_simulation(svc, a),
        "list_simulations" => Ok(json!({ "simulations": to_json(&svc.list()) })),
        "get_status" => {
            let view = svc.status(&id_argument(&a)?, recent_logs_argument(&a))?;
            Ok(apply_include_series(to_json(&view), &a))
        }
        "get_logs" => get_logs(svc, &a),
        "get_results" => get_results(svc, &a),
        "cancel_simulation" => Ok(to_json(&svc.cancel(&id_argument(&a)?)?)),
        "delete_simulation" => {
            let id = id_argument(&a)?;
            svc.jobs.delete(&id)?;
            Ok(json!({"deleted": id}))
        }
        _ => Err(ApiError::bad_request(format!("unknown tool {name:?}"))),
    }
}

/// `start_simulation`: the arguments are a [`SimulationRequest`].
fn start_simulation(svc: &Service, a: Value) -> Result<Value, ApiError> {
    let req: SimulationRequest = serde_json::from_value(a)
        .map_err(|e| ApiError::bad_request(format!("invalid arguments: {e}")))?;
    let view = svc.start(req)?;
    let mut out = to_json(&view);
    out["next"] = json!(format!(
        "Follow it with wait_for_simulation {{\"id\": \"{}\"}} or get_status; read the results with get_results once status is completed.",
        view.id
    ));
    Ok(out)
}

/// `wait_for_simulation`.
async fn wait_for_simulation(svc: &Service, a: &Value) -> Result<Value, ApiError> {
    let until = match a.get("until").and_then(Value::as_str) {
        None | Some("change") => Until::Change,
        Some("done") => Until::Done,
        Some(u) => {
            return Err(ApiError::bad_request(format!(
                "until: expected change or done, got {u:?}"
            )));
        }
    };
    let secs = a
        .get("timeout_secs")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite())
        .unwrap_or(DEFAULT_WAIT_SECS)
        .clamp(0.0, MAX_WAIT.as_secs_f64());
    let after = a.get("after_version").and_then(Value::as_u64);
    let view = svc
        .wait(
            &id_argument(a)?,
            after,
            until,
            Duration::from_secs_f64(secs),
            recent_logs_argument(a),
        )
        .await?;
    Ok(apply_include_series(to_json(&view), a))
}

/// `get_logs`.
fn get_logs(svc: &Service, a: &Value) -> Result<Value, ApiError> {
    let since = a.get("since").and_then(Value::as_u64).unwrap_or(0);
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LOG_LIMIT) as usize;
    let level = Level::from_query(a.get("min_level").and_then(Value::as_str))
        .map_err(|e| ApiError::bad_request(format!("min_level: {e}")))?;
    Ok(to_json(&svc.logs(&id_argument(a)?, since, limit, level)?))
}

/// `get_results`: the summary and rows as JSON, or `measurements.csv` (`format: csv`).
fn get_results(svc: &Service, a: &Value) -> Result<Value, ApiError> {
    let series: Vec<SeriesSelector> = match a.get("series") {
        Some(s) => serde_json::from_value(s.clone())
            .map_err(|e| ApiError::bad_request(format!("series: {e}")))?,
        None => Vec::new(),
    };
    if a.get("format").and_then(Value::as_str) == Some("csv") {
        let mut csv = svc.csv(&id_argument(a)?, &series)?;
        let max_bytes = a
            .get("max_csv_bytes")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_MAX_CSV_BYTES, |n| n as usize);
        let truncated = truncate_to_whole_lines(&mut csv, max_bytes);
        return Ok(json!({"format": "csv", "truncated": truncated, "csv": csv}));
    }
    let query = ResultsQuery {
        include_rows: a
            .get("include_rows")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        series,
        max_rows_per_series: a
            .get("max_rows_per_series")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
    };
    svc.results(&id_argument(a)?, &query)
}

/// The required argument `id`, the simulation id.
fn id_argument(a: &Value) -> Result<String, ApiError> {
    a.get("id")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| ApiError::bad_request("argument `id` (the simulation id) is required"))
}

/// The argument `recent_logs`.
fn recent_logs_argument(a: &Value) -> usize {
    a.get("recent_logs")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_RECENT_LOGS, |n| n as usize)
}

/// `include_series: false` drops the per-series statistics from a status (a shorter answer).
fn apply_include_series(view: Value, args: &Value) -> Value {
    if args.get("include_series").and_then(Value::as_bool) == Some(false) {
        return without_series(view);
    }
    view
}

/// Cuts `text` to at most `max_bytes`, at the end of a line; returns whether it was cut.
fn truncate_to_whole_lines(text: &mut String, max_bytes: usize) -> bool {
    if text.len() <= max_bytes {
        return false;
    }
    let mut cut = max_bytes;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut = text[..cut].rfind('\n').map_or(0, |i| i + 1);
    text.truncate(cut);
    true
}

/// The schema of a simulation id argument.
fn id_schema() -> Value {
    json!({"type": "string", "description": "The simulation id returned by start_simulation (e.g. sim-1-0a3f2c)."})
}

/// The schema of a series selector.
fn selector_schema() -> Value {
    json!({
        "type": "object",
        "description": "Selects series whose measuring point and metric contain these substrings.",
        "properties": {
            "measuring_point": {"type": "string", "description": "Substring of the measuring point, e.g. a usage scenario, operation or resource name."},
            "metric": {"type": "string", "description": "Substring of the metric, e.g. \"Response Time\", \"Utilization\"."}
        },
        "additionalProperties": false
    })
}

/// The schema of a termination condition, with `all`/`any` nested `depth` levels.
fn condition_schema(depth: u32) -> Value {
    let mut props = json!({
        "timeout_secs": {"type": "number", "exclusiveMinimum": 0, "description": "Wall-clock seconds since the run started."},
        "max_measurements": {"type": "integer", "minimum": 0, "description": "Finished usage-scenario runs (SimuLizar's measurement count). Unlike config.max_measurements, -1 is not off: leave the criterion out."},
        "max_tuples": {"type": "integer", "minimum": 0, "description": "Recorded measurement tuples, all series together."},
        "max_events": {"type": "integer", "minimum": 0, "description": "Processed simulation events."},
        "max_sim_time": {"type": "number", "minimum": 0, "description": "Simulated seconds."},
        "confidence": {
            "type": "object",
            "description": "Stops when the batch-means confidence interval of the mean of every watched series (default: all response-time series with data) is narrow enough.",
            "properties": {
                "level": {"type": "number", "exclusiveMinimum": 0, "exclusiveMaximum": 1, "default": DEFAULT_CONFIDENCE_LEVEL},
                "relative_half_width": {"type": "number", "exclusiveMinimum": 0, "description": "Target half width relative to the mean, e.g. 0.05 for ±5 % (the default when no target is given)."},
                "absolute_half_width": {"type": "number", "exclusiveMinimum": 0, "description": "Target half width in the series' unit (seconds for response times)."},
                "series": {"type": "array", "items": selector_schema(), "description": "Series to watch; default: every response-time series."},
                "batch_size": {"type": "integer", "minimum": 1, "description": format!("Fixed batch size (SimuCom's static batch means); default: {DEFAULT_MIN_BATCHES}-{} batches whose size doubles as the series grows.", 2 * DEFAULT_MIN_BATCHES)},
                "min_batches": {"type": "integer", "minimum": MIN_BATCHES_FOR_INTERVAL, "default": DEFAULT_MIN_BATCHES},
                "min_samples": {"type": "integer", "minimum": 0, "default": DEFAULT_MIN_SAMPLES, "description": "Values every watched series needs first."}
            },
            "additionalProperties": false
        },
        "combine": {"type": "string", "enum": ["any", "all"], "default": "any", "description": "Whether any or all criteria of this object must hold."}
    });
    if depth > 0 {
        props["all"] = json!({"type": "array", "minItems": 1, "items": condition_schema(depth - 1), "description": "Holds when every listed condition holds."});
        props["any"] = json!({"type": "array", "minItems": 1, "items": condition_schema(depth - 1), "description": "Holds when any listed condition holds."});
    }
    json!({"type": "object", "properties": props, "additionalProperties": false})
}

/// The tool definitions (`tools/list`).
pub fn tools() -> Vec<Value> {
    let ro = json!({"readOnlyHint": true, "openWorldHint": false});
    vec![
        json!({
            "name": "server_info",
            "title": "Server information",
            "description": "SimOxide server version, available flavours and modes, build features (scaling policies, reconfiguration, exact mode) and limits.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": ro,
        }),
        json!({
            "name": "list_models",
            "title": "List server-side models",
            "description": "Model directories on the server (below its --allow-dir directories) that start_simulation accepts as model_dir, with their root and files.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": ro,
        }),
        json!({
            "name": "start_simulation",
            "title": "Start a simulation",
            "description": "Submits a Palladio model (repository, system, resource environment, allocation, usage model, measuring points and monitor repository, optionally run.json, scaling policies .spd, HPA autoscalers .hpa.json, load profiles .loadprofile.json) and starts simulating it. Returns the simulation id at once; the run goes on in the background. Give either files or model_dir. The run needs a bounding stop: config.max_sim_time / config.max_measurements (the simulator's own stop conditions, as run.json) or a termination criterion such as timeout_secs or max_measurements; a confidence criterion alone ends at the server's run-time cap at the latest. When the server already runs and queues as many simulations as it may, the call fails: try again once one has ended.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "A label for listings."},
                    "files": {
                        "type": "array",
                        "description": "The model files. Names are relative paths as the files' hrefs refer to each other (usually bare file names).",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string", "description": "e.g. ms.repository, refsim.measuringpoint, refsim.monitorrepository, run.json"},
                                "content": {"type": "string", "description": "The file content as text (XMI or JSON)."},
                                "content_base64": {"type": "string", "description": "The content base64-encoded, instead of content."}
                            },
                            "required": ["name"],
                            "additionalProperties": false
                        }
                    },
                    "model_dir": {"type": "string", "description": "A model directory on the server instead of files (see list_models)."},
                    "root": {"type": "string", "description": "The allowed directory model_dir is below, as list_models reports it (its root). Needed when several have a model of that name."},
                    "config": {
                        "type": "object",
                        "description": "Run configuration; overrides a run.json among the files.",
                        "properties": {
                            "seed": {"type": "integer"},
                            "flavour": {"type": "string", "enum": ["simulizar", "slingshot", "slingshot-exact"], "default": "simulizar"},
                            "mode": {"type": "string", "enum": ["fast", "exact"], "default": "fast", "description": "exact needs a server built with the exact feature."},
                            "quirks": {"type": "string", "description": "Slingshot compat flags: none (default), all, Q1,Q5,..."},
                            "max_sim_time": {"type": "integer", "description": "Simulator stop: simulated seconds (-1 = off). Required for slingshot-exact."},
                            "max_measurements": {"type": "integer", "description": "Simulator stop: finished usage-scenario runs (-1 = off)."},
                            "usagemodel": {"type": "string"},
                            "allocation": {"type": "array", "items": {"type": "string"}},
                            "monitorrepository": {"type": "string"},
                            "simulate_linking_resources": {"type": "boolean"},
                            "simulate_throughput_of_linking_resources": {"type": "boolean"},
                            "ps_algorithm": {"type": "string", "enum": ["exact", "virtual-time"]},
                            "usage_streams": {"type": "boolean"},
                            "fix_double_resume": {"type": "boolean"},
                            "spd": {"type": "boolean", "description": "Run the model's scaling policies (default true)."},
                            "spd_quirks": {"type": "string"},
                            "spd_log_all": {"type": "boolean"},
                            "spd_decisions": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}, "description": "Scaling decisions: lists of policy ids or names. The run stops before each scaling policy's adjustment and resolves decision i with the i-th list, whose adjustments run instead ([] declines); at a decision beyond the lists it ends with stop reason 'decision'. Results: spd.decisions, spd.pending_decision."},
                            "hpa": {"type": "boolean", "description": "Run *.hpa.json autoscalers (default true)."},
                            "load_profile": {"type": "boolean", "description": "Apply *.loadprofile.json files (default true)."},
                            "max_events_per_instant": {"type": "integer", "minimum": 0},
                            "max_stack_depth": {"type": "integer", "minimum": 0},
                            "max_processes": {"type": "integer", "minimum": 0},
                            "max_steps": {"type": "integer", "minimum": 0}
                        },
                        "additionalProperties": false
                    },
                    "termination": {
                        "description": "When the server stops the run, checked between events. The criteria of one object combine with `combine` (default any); `all`/`any` nest groups. Example: {\"timeout_secs\": 120, \"all\": [{\"confidence\": {\"relative_half_width\": 0.05}}, {\"max_measurements\": 1000}]}.",
                        "allOf": [condition_schema(CONDITION_SCHEMA_DEPTH)]
                    },
                    "progress": {
                        "type": "object",
                        "properties": {
                            "interval_ms": {"type": "integer", "minimum": MIN_PROGRESS_INTERVAL_MS, "maximum": MAX_PROGRESS_INTERVAL_MS, "default": DEFAULT_PROGRESS_INTERVAL_MS, "description": "Wall-clock interval of progress updates."},
                            "warmup_time": {"type": "number", "minimum": 0, "description": "Ignore rows before this simulated time in statistics and confidence intervals."},
                            "warmup_samples": {"type": "integer", "minimum": 0, "description": "Ignore the first rows of every series."}
                        },
                        "additionalProperties": false
                    }
                },
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false},
        }),
        json!({
            "name": "get_status",
            "title": "Simulation status and live progress",
            "description": "Status of a simulation (queued, loading, running, completed, cancelled, failed) with its latest progress: simulation time, events, finished usage-scenario runs, speed, the state of every termination criterion, and per-series statistics (count, mean, std_dev, min, max, time-weighted mean, confidence interval for watched series); plus the newest log entries.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id_schema(),
                    "recent_logs": {"type": "integer", "minimum": 0, "default": DEFAULT_RECENT_LOGS},
                    "include_series": {"type": "boolean", "default": true, "description": "false leaves out the per-series statistics."}
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": ro,
        }),
        json!({
            "name": "wait_for_simulation",
            "title": "Wait for progress or the end",
            "description": format!(
                "Blocks until the simulation changes (until=change: new progress, log entry or status; default) or ends (until=done), at most timeout_secs (default {DEFAULT_WAIT_SECS}, max {}), then returns what get_status returns. Pass the returned version as after_version to wait for the next change.",
                MAX_WAIT.as_secs()
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id_schema(),
                    "until": {"type": "string", "enum": ["change", "done"], "default": "change"},
                    "timeout_secs": {"type": "number", "minimum": 0, "maximum": MAX_WAIT.as_secs(), "default": DEFAULT_WAIT_SECS},
                    "after_version": {"type": "integer", "minimum": 0, "description": "Wait for a version above this (default: the current one)."},
                    "recent_logs": {"type": "integer", "minimum": 0, "default": DEFAULT_RECENT_LOGS},
                    "include_series": {"type": "boolean", "default": true}
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": ro,
        }),
        json!({
            "name": "get_logs",
            "title": "Simulation log",
            "description": "Log entries of a simulation newer than `since`: model loading and compile warnings, scaling-policy (SPD/HPA) decisions and adjustments with their simulation time, termination and errors. Pass the returned last_seq as since to read only new entries.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id_schema(),
                    "since": {"type": "integer", "minimum": 0, "default": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LOG_PAGE, "default": DEFAULT_LOG_LIMIT},
                    "min_level": {"type": "string", "enum": ["debug", "info", "warn", "error"], "default": "info"}
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": ro,
        }),
        json!({
            "name": "get_results",
            "title": "Simulation results",
            "description": "Final results of an ended simulation (completed, cancelled, or failed after it started): stop reason, simulation time, events, usage-scenario runs, and per-series statistics with percentiles (p50/p90/p95/p99) and confidence intervals. include_rows adds the raw (time, value) rows, at most max_rows_per_series per series (evenly spaced; see its default, 0 = all). format=csv returns measurements.csv instead (truncated to max_csv_bytes).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id_schema(),
                    "series": {"type": "array", "items": selector_schema(), "description": "Only these series (default all)."},
                    "include_rows": {"type": "boolean", "default": false},
                    "max_rows_per_series": {"type": "integer", "minimum": 0, "default": DEFAULT_MAX_ROWS_PER_SERIES},
                    "format": {"type": "string", "enum": ["json", "csv"], "default": "json"},
                    "max_csv_bytes": {"type": "integer", "minimum": 0, "default": DEFAULT_MAX_CSV_BYTES}
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": ro,
        }),
        json!({
            "name": "cancel_simulation",
            "title": "Stop a simulation",
            "description": "Stops a queued or running simulation; it ends as cancelled and its results so far stay readable with get_results.",
            "inputSchema": {"type": "object", "properties": {"id": id_schema()}, "required": ["id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "list_simulations",
            "title": "List simulations",
            "description": "Every simulation the server keeps, oldest first, with status, stop reason and the latest simulation time.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": ro,
        }),
        json!({
            "name": "delete_simulation",
            "title": "Delete a simulation",
            "description": "Cancels the simulation if it runs and forgets it with its results.",
            "inputSchema": {"type": "object", "properties": {"id": id_schema()}, "required": ["id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false},
        }),
    ]
}

/// Serves MCP over stdin and stdout until stdin closes, then answers the requests still in
/// progress. Diagnostics go to stderr.
pub fn serve_stdio(mcp: Arc<Mcp>) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let send = |out: &Mutex<std::io::Stdout>, v: &Value| {
        let mut o = lock_ignoring_poison(out);
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    };
    let stdin = std::io::stdin();
    let mut in_progress = Vec::new();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(
                    &out,
                    &rpc_error(Value::Null, RPC_PARSE_ERROR, format!("parse error: {e}")),
                );
                continue;
            }
        };
        let (m, o) = (mcp.clone(), out.clone());
        // concurrently: a waiting request must not hold up the others
        in_progress.push(runtime.spawn(async move {
            if let Some(reply) = m.handle_message(msg).await {
                // writing to stdout can block on a slow client: off the async workers
                let _ = tokio::task::spawn_blocking(move || send(&o, &reply)).await;
            }
        }));
        in_progress.retain(|h| !h.is_finished());
    }
    runtime.block_on(async {
        for h in in_progress {
            let _ = h.await;
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp() -> Mcp {
        let jobs = crate::job::JobManager::new(crate::job::ServerConfig::default());
        Mcp {
            svc: Arc::new(Service { jobs }),
        }
    }

    #[tokio::test]
    async fn initialize_and_list() {
        let m = mcp();
        let r = m
            .handle(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}))
            .await
            .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert!(
            m.handle(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .await
                .is_none()
        );
        let r = m
            .handle(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
            .await
            .unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"start_simulation"));
        assert!(names.contains(&"get_results"));
    }

    #[tokio::test]
    async fn errors_are_tool_results() {
        let m = mcp();
        let r = m
            .handle(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "get_status", "arguments": {"id": "nope"}}}))
            .await
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let r = m
            .handle(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "start_simulation", "arguments": {"files": []}}}))
            .await
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let r = m
            .handle(json!({"jsonrpc": "2.0", "id": 5, "method": "nope"}))
            .await
            .unwrap();
        assert_eq!(r["error"]["code"], RPC_METHOD_NOT_FOUND);
    }
}
