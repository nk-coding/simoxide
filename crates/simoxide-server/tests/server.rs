//! End-to-end tests of the HTTP API and the MCP endpoint on corpus models, in process (the
//! router answers requests without a socket).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use simoxide_server::http::{HttpConfig, router};
use simoxide_server::{JobManager, ServerConfig, Service};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn app_with(cfg: HttpConfig) -> Router {
    app_of(
        ServerConfig {
            allow_dirs: vec![repo().join("corpus"), repo().join("corpus-spd")],
            max_concurrent: 4,
            ..ServerConfig::default()
        },
        cfg,
    )
}

fn app_of(server: ServerConfig, cfg: HttpConfig) -> Router {
    router(
        Arc::new(Service {
            jobs: JobManager::new(server),
        }),
        cfg,
    )
}

/// The status and body of a request built by the caller (for custom headers).
async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (s, String::from_utf8_lossy(&bytes).into_owned())
}

/// A JSON-RPC `tools/call` request.
fn tool_call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
           "params": {"name": name, "arguments": args}})
}

fn app() -> Router {
    app_with(HttpConfig::default())
}

/// The files of a model directory as request files (recursively, without `expected/`).
fn files_of(dir: &Path) -> Vec<Value> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<Value>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                if name != "expected" {
                    walk(root, &p, out);
                }
            } else if !name.ends_with(".md") && !name.ends_with(".txt") {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(json!({"name": rel, "content": std::fs::read_to_string(&p).unwrap()}));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (s, text) = call_text(app, method, uri, body, None).await;
    (
        s,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn call_text(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> (StatusCode, String) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (s, String::from_utf8_lossy(&bytes).into_owned())
}

/// Starts a simulation and waits for its end; its final state.
async fn run(app: &Router, req: Value) -> Value {
    let (s, v) = call(app, "POST", "/v1/simulations", Some(req)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let (s, v) = call(
        app,
        "GET",
        &format!("/v1/simulations/{id}?wait=120&until=done&logs=50"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        ["completed", "cancelled", "failed"].contains(&v["status"].as_str().unwrap()),
        "{v}"
    );
    v
}

async fn results(app: &Router, id: &str, query: &str) -> Value {
    let (s, v) = call(
        app,
        "GET",
        &format!("/v1/simulations/{id}/results{query}"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn uploaded_model_gives_the_library_result() {
    let app = app();
    let dir = repo().join("corpus/x_sl_mediastore");
    let v = run(
        &app,
        json!({"files": files_of(&dir), "config": {"seed": 3, "max_measurements": 300}}),
    )
    .await;
    assert_eq!(v["status"], "completed", "{v}");
    assert_eq!(v["stop_reason"]["kind"], "simulator");
    assert_eq!(v["model"]["flavour"], "simulizar");
    let id = v["id"].as_str().unwrap();
    let (s, csv) = call_text(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/measurements.csv"),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // the same run through the library
    let mut spec = simoxide_sim::RunSpec::load(&dir.join("run.json")).unwrap();
    spec.seed = 3;
    spec.max_measurements = 300;
    let files: Vec<(String, Vec<u8>)> = files_of(&dir)
        .iter()
        .map(|f| {
            (
                f["name"].as_str().unwrap().to_string(),
                f["content"].as_str().unwrap().as_bytes().to_vec(),
            )
        })
        .collect();
    let model = spec.load_model_memory(&files).unwrap();
    let cm = simoxide_sim::CompiledModel::compile(model).unwrap();
    let mut cfg = spec.sim_config("");
    cfg.mode = simoxide_sim::Mode::Fast;
    let r = simoxide_sim::run(&cm, cfg, simoxide_sim::Outputs::default()).unwrap();
    assert_eq!(csv, r.measurements.to_csv());
    let res = results(&app, id, "").await;
    assert_eq!(res["summary"]["measurements"], 300);
    assert_eq!(res["summary"]["events"], r.events);
    // one interval from time 0: the cursor at its end is the number of rows
    assert_eq!(res["summary"]["tuples"], csv.lines().count() as u64 - 1);
    let series = res["summary"]["series"].as_array().unwrap();
    assert!(!series.is_empty());
    assert!(series.iter().all(|s| s["percentiles"]["p50"].is_number()));
}

#[tokio::test(flavor = "multi_thread")]
async fn termination_by_count_confidence_and_both() {
    let app = app();
    // a count: the simulator's own stop is off, the server stops the run
    let v = run(
        &app,
        json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
               "termination": {"max_measurements": 1000}}),
    )
    .await;
    assert_eq!(v["stop_reason"]["kind"], "termination", "{v}");
    let n = v["progress"]["measurements"].as_i64().unwrap();
    assert!((1000..=1001).contains(&n), "{n}");

    // a confidence interval
    let v = run(
        &app,
        json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
               "termination": {"timeout_secs": 60,
                   "confidence": {"relative_half_width": 0.05, "series": [{"metric": "Response Time"}]}}}),
    )
    .await;
    assert_eq!(v["stop_reason"]["kind"], "termination", "{v}");
    assert!(
        v["stop_reason"]["detail"]
            .as_str()
            .unwrap()
            .contains("confidence")
    );
    let id = v["id"].as_str().unwrap();
    let res = results(&app, id, "?metric=Response").await;
    let series = res["summary"]["series"].as_array().unwrap();
    assert!(!series.is_empty());
    for s in series.iter().filter(|s| s["count"].as_u64() > Some(0)) {
        let c = &s["confidence"];
        assert_eq!(c["satisfied"], true, "{s}");
        assert!(c["interval"]["relative_half_width"].as_f64().unwrap() <= 0.05);
    }

    // both: the interval and at least 5000 runs
    let v = run(
        &app,
        json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
               "termination": {"timeout_secs": 60,
                   "all": [{"confidence": {"relative_half_width": 0.05}}, {"max_measurements": 5000}]}}),
    )
    .await;
    assert_eq!(v["stop_reason"]["kind"], "termination", "{v}");
    let d = v["stop_reason"]["detail"].as_str().unwrap();
    assert!(
        d.starts_with("all(") && d.contains("confidence") && d.contains("max_measurements"),
        "{d}"
    );
    assert!(v["progress"]["measurements"].as_i64().unwrap() >= 5000);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_and_cancel_keep_results() {
    let app = app();
    let t0 = std::time::Instant::now();
    let v = run(
        &app,
        json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
               "termination": {"timeout_secs": 0.3}, "progress": {"interval_ms": 50}}),
    )
    .await;
    assert!(t0.elapsed().as_secs_f64() < 5.0);
    assert_eq!(v["status"], "completed");
    assert!(
        v["stop_reason"]["detail"]
            .as_str()
            .unwrap()
            .contains("timeout"),
        "{v}"
    );
    assert_eq!(v["results_available"], true);

    let (_, v) = call(
        &app,
        "POST",
        "/v1/simulations",
        Some(
            json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
                    "termination": {"timeout_secs": 60}, "progress": {"interval_ms": 50}}),
        ),
    )
    .await;
    let id = v["id"].as_str().unwrap().to_string();
    // wait for live progress first
    let (_, v) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}?wait=10&after_version=4"),
        None,
    )
    .await;
    assert!(
        ["loading", "running"].contains(&v["status"].as_str().unwrap()),
        "{v}"
    );
    let (s, _) = call(&app, "GET", &format!("/v1/simulations/{id}/results"), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(&app, "POST", &format!("/v1/simulations/{id}/cancel"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}?wait=30&until=done"),
        None,
    )
    .await;
    assert_eq!(v["status"], "cancelled", "{v}");
    let res = results(&app, &id, "?rows=true&max_rows=5").await;
    assert!(res["summary"]["tuples"].as_u64().unwrap() > 0);
    for r in res["rows"].as_array().unwrap() {
        assert!(r["rows"].as_array().unwrap().len() <= 5);
    }
    let (s, _) = call(&app, "DELETE", &format!("/v1/simulations/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "GET", &format!("/v1/simulations/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_requests_are_rejected() {
    let app = app();
    for (req, what) in [
        (json!({}), "no model"),
        (
            json!({"model_dir": "x_sl_mediastore", "termination": {"timeout": 1}}),
            "unknown field",
        ),
        (
            json!({"model_dir": "../crates"}),
            "outside the allowed directories",
        ),
        (
            json!({"files": [{"name": "../x.repository", "content": ""}]}),
            "bad file name",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"flavour": "slingshot-exact"}}),
            "slingshot-exact needs a time",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"mode": "warp"}}),
            "bad mode",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "termination": {"confidence": {"level": 2}}}),
            "bad level",
        ),
        // entry files outside the model directory
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"usagemodel": "/etc/passwd"}}),
            "absolute usagemodel",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"allocation": ["../x_sl_loadbalancer/a.allocation"]}}),
            "allocation with ..",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"monitorrepository": "/tmp/m.monitorrepository"}}),
            "absolute monitorrepository",
        ),
        // checks that need no model, made before the job exists
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"quirks": "all"}}),
            "quirks without slingshot",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "config": {"spd_quirks": "bogus"}}),
            "bad spd_quirks",
        ),
        (
            json!({"model_dir": "x_sl_loadbalancer", "config": {"flavour": "slingshot-exact", "max_sim_time": 10, "usage_streams": true}}),
            "usage streams in slingshot-exact",
        ),
        (
            json!({"files": [{"name": "a.usagemodel", "content_base64": "not base64!"}]}),
            "bad base64",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "termination": {"max_measurements": -1}}),
            "negative count",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "termination": {"combine": "all", "max_events": 1000, "all": []}}),
            "empty all",
        ),
        (
            json!({"model_dir": "x_sl_mediastore", "progress": {"interval_ms": u64::MAX}}),
            "interval beyond the clock",
        ),
        (
            json!({"files": [{"name": "a.usagemodel", "content": ""}], "root": "corpus"}),
            "root without model_dir",
        ),
    ] {
        let (s, v) = call(&app, "POST", "/v1/simulations", Some(req)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {v}");
        assert!(v["error"].is_string(), "{what}: {v}");
    }
    let (s, _) = call(&app, "GET", "/v1/simulations/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // a model that does not load: a failed job with the loader's message
    let v = run(
        &app,
        json!({"files": [{"name": "a.usagemodel", "content": "<not xmi"}], "config": {"max_measurements": 1}}),
    )
    .await;
    assert_eq!(v["status"], "failed");
    assert!(v["error"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn server_sent_events_carry_progress_logs_and_the_end() {
    let app = app();
    let (_, v) = call(
        &app,
        "POST",
        "/v1/simulations",
        Some(
            json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
                    "termination": {"timeout_secs": 0.5}, "progress": {"interval_ms": 50}}),
        ),
    )
    .await;
    let id = v["id"].as_str().unwrap();
    let (s, body) = call_text(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/events"),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    for ev in [
        "event: status",
        "event: progress",
        "event: log",
        "event: done",
    ] {
        assert!(body.contains(ev), "{ev} missing in\n{body}");
    }
    assert!(
        body.trim_end()
            .rsplit("\n\n")
            .next()
            .unwrap()
            .contains("event: done")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_over_http() {
    let app = app();
    let rpc = |id: u64, method: &str, params: Value| json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let (s, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(rpc(1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}))),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    let (s, _) = call(
        &app,
        "POST",
        "/mcp",
        Some(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let tool = |id: u64, name: &str, args: Value| {
        rpc(id, "tools/call", json!({"name": name, "arguments": args}))
    };
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool(
            2,
            "start_simulation",
            json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": 200}}),
        )),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    let id = v["result"]["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool(
            3,
            "wait_for_simulation",
            json!({"id": id, "until": "done", "timeout_secs": 60, "include_series": false}),
        )),
    )
    .await;
    let st = &v["result"]["structuredContent"];
    assert_eq!(st["status"], "completed", "{v}");
    assert!(st["progress"].get("series").is_none());
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool(
            4,
            "get_results",
            json!({"id": id, "include_rows": true, "max_rows_per_series": 2}),
        )),
    )
    .await;
    let r = &v["result"]["structuredContent"];
    assert_eq!(r["summary"]["measurements"], 200, "{v}");
    assert!(
        r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["rows"].as_array().unwrap().len() <= 2)
    );
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool(
            5,
            "get_results",
            json!({"id": id, "format": "csv", "max_csv_bytes": 200}),
        )),
    )
    .await;
    let r = &v["result"]["structuredContent"];
    assert_eq!(r["truncated"], true);
    assert!(
        r["csv"]
            .as_str()
            .unwrap()
            .starts_with("measuring_point,metric,time,value\n")
    );
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool(6, "get_logs", json!({"id": id}))),
    )
    .await;
    assert!(
        !v["result"]["structuredContent"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // a batch
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(json!([
            rpc(7, "ping", json!({})),
            tool(8, "list_simulations", json!({}))
        ])),
    )
    .await;
    assert_eq!(v.as_array().unwrap().len(), 2);
    let (s, _) = call(&app, "GET", "/mcp", None).await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test(flavor = "multi_thread")]
async fn scaling_policies_log_their_decisions() {
    let app = app();
    let dir = repo().join("corpus-spd/rmuc_app_metrics");
    let files = files_of(&dir);
    assert!(
        files
            .iter()
            .any(|f| f["name"] == "monitors/monitors.monitorrepository")
    );
    let v = run(
        &app,
        json!({"files": files, "config": {"flavour": "slingshot", "max_sim_time": 300}}),
    )
    .await;
    assert_eq!(v["status"], "completed", "{v}");
    assert!(v["model"]["scaling_policies"].as_u64().unwrap() > 0);
    let id = v["id"].as_str().unwrap();
    let (_, logs) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/logs?limit=10000"),
        None,
    )
    .await;
    let spd: Vec<&Value> = logs["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source"] == "spd")
        .collect();
    assert!(spd.iter().any(|e| e["data"]["k"] == "policy"));
    assert!(
        spd.iter()
            .any(|e| e["data"]["k"] == "adjust" && e["sim_time"].is_number())
    );
    let res = results(&app, id, "").await;
    assert!(res["summary"]["spd"]["requests"].as_u64().unwrap() > 0);
}

/// `config.runtime_metadata` (issue #9, `simoxide run --model-metadata --runtime-metadata`):
/// the model file and the runtime file of the run through HTTP and MCP. The runtime file starts
/// with the model file's deployment, has one change per executed adjustment (at the times of the
/// decision log's `adjust` records), and describes every series of `measurements.csv`.
#[tokio::test(flavor = "multi_thread")]
async fn runtime_metadata_of_a_run() {
    let app = app();
    let files = files_of(&repo().join("corpus-spd/rmuc_app_metrics"));
    let req = |cfg: Value| json!({"files": files, "config": cfg});
    let v = run(
        &app,
        req(json!({"flavour": "slingshot", "max_sim_time": 300, "runtime_metadata": true})),
    )
    .await;
    assert_eq!(v["status"], "completed", "{v}");
    let id = v["id"].as_str().unwrap();
    let (s, meta) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/runtime-metadata"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{meta}");
    let (model, rt) = (&meta["model"], &meta["runtime"]);
    assert_eq!(model["format"], "simoxide-model");
    assert_eq!(rt["format"], "simoxide-runtime");
    assert_eq!(rt["model"]["hash"], model["hash"]);
    assert!(model["scaling"]["policies"].as_array().unwrap().len() > 0);
    assert_eq!(rt["start"]["time_ns"], 0);
    assert_eq!(rt["start"]["deployment"], model["deployment"]);
    assert_eq!(rt["run"]["status"], "finished");
    assert_eq!(rt["end"]["time_ns"], 300_000_000_000i64);
    // one change per adjustment of the decision log, at its time, caused by its policy
    let (_, logs) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/logs?limit=100000"),
        None,
    )
    .await;
    let adjusts: Vec<&Value> = logs["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source"] == "spd" && e["data"]["k"] == "adjust")
        .collect();
    let changes = rt["changes"].as_array().unwrap();
    assert!(!changes.is_empty());
    assert_eq!(changes.len(), adjusts.len(), "{}", rt["changes"]);
    for (c, a) in changes.iter().zip(&adjusts) {
        assert_eq!(c["time"], a["sim_time"], "{c}");
        assert_eq!(c["cause"]["type"], "policy");
        assert_eq!(c["cause"]["policy"], a["data"]["policy"], "{c}");
    }
    // a build that executes adjustments changes the deployment (the service group's
    // assemblies); one that does not reports every adjustment as failed, with no change
    let executed = v["model"]["adjustments_executed"] == true;
    assert_eq!(
        changes
            .iter()
            .any(|c| c["added"] != json!({}) || c["removed"] != json!({})),
        executed,
        "{}",
        rt["changes"]
    );
    if !executed {
        assert!(changes.iter().all(|c| c["outcome"]["ok"] == false));
    }
    // every series of measurements.csv has its metadata
    let (_, csv) = call_text(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/measurements.csv"),
        None,
        None,
    )
    .await;
    for line in csv.lines().skip(1) {
        let mut f = line.split(',');
        let (mp, metric) = (f.next().unwrap(), f.next().unwrap());
        assert!(
            rt["series"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["measuring_point"] == mp
                    && s["metric"] == metric
                    && s["unit"].is_string()),
            "no metadata for ({mp}, {metric})"
        );
    }
    // MCP: the same
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(1, "get_runtime_metadata", json!({"id": id}))),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    assert_eq!(v["result"]["structuredContent"], meta);
    // without the option: no metadata; slingshot-exact: refused
    let v = run(
        &app,
        req(json!({"flavour": "slingshot", "max_sim_time": 10})),
    )
    .await;
    let (s, _) = call(
        &app,
        "GET",
        &format!(
            "/v1/simulations/{}/runtime-metadata",
            v["id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(
        &app,
        "POST",
        "/v1/simulations",
        Some(req(
            json!({"flavour": "slingshot-exact", "max_sim_time": 10, "runtime_metadata": true}),
        )),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// `config.spd_decisions` (issue #6, `simoxide run --pause-on-decision` / `resume --decide` in
/// one job): no list stops at the first recommendation, un-applied; a list resolves it (the
/// recommendation, declined) and the run goes on to the next decision; through HTTP and MCP.
#[tokio::test(flavor = "multi_thread")]
async fn scaling_decisions_from_the_request() {
    let app = app();
    let files = files_of(&repo().join("corpus-spd/rmuc_app_metrics"));
    let req = |plan: Value| json!({"files": files, "config": {"flavour": "slingshot", "max_sim_time": 300, "spd_decisions": plan}});
    // no list: the first recommendation is pending, nothing adjusted
    let v = run(&app, req(json!([]))).await;
    assert_eq!(v["status"], "completed", "{v}");
    assert_eq!(v["stop_reason"]["kind"], "decision", "{v}");
    let spd = results(&app, v["id"].as_str().unwrap(), "").await["summary"]["spd"].clone();
    assert_eq!(spd["decisions"], json!([]));
    let first = spd["pending_decision"].clone();
    assert_eq!(first["decision"], 1, "{spd}");
    let t1 = first["time_ns"].as_i64().unwrap();
    assert!(t1 > 0 && t1 <= 300_000_000_000, "{first}");
    let recommended: Vec<Value> = first["recommended"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["policy"].clone())
        .collect();
    assert!(!recommended.is_empty());
    assert_eq!(v["progress"]["sim_time"].as_f64().unwrap(), t1 as f64 / 1e9);

    // the recommendation, then declined: resolved in order, each with its outcomes
    let v = run(&app, req(json!([recommended, []]))).await;
    assert_eq!(v["status"], "completed", "{v}");
    let spd = results(&app, v["id"].as_str().unwrap(), "").await["summary"]["spd"].clone();
    let d = spd["decisions"].as_array().unwrap();
    assert_eq!(d.len(), 2, "{spd}");
    assert_eq!(d[0]["decision"], 1);
    assert_eq!(d[0]["time_ns"], t1);
    assert_eq!(d[0]["recommended"], first["recommended"]);
    assert_eq!(d[0]["selected"], json!(recommended));
    assert_eq!(
        d[0]["outcomes"].as_array().unwrap().len(),
        recommended.len()
    );
    assert_eq!(d[1]["decision"], 2);
    assert_eq!(d[1]["selected"], json!([]));
    assert_eq!(d[1]["outcomes"], json!([]));
    assert!(d[1]["time_ns"].as_i64().unwrap() >= t1);
    match spd["pending_decision"]["decision"].as_u64() {
        Some(n) => assert_eq!(
            (n, v["stop_reason"]["kind"].as_str()),
            (3, Some("decision"))
        ),
        None => assert_eq!(v["stop_reason"]["kind"], "simulator", "{v}"),
    }
    let id = v["id"].as_str().unwrap();
    let (_, logs) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/logs?limit=10000"),
        None,
    )
    .await;
    let kinds: Vec<&str> = logs["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source"] == "spd")
        .filter_map(|e| e["data"]["k"].as_str())
        .filter(|k| ["decision", "select"].contains(k))
        .collect();
    assert_eq!(&kinds[..4], ["decision", "select", "decision", "select"]);

    // MCP: the same option of start_simulation
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(1, "start_simulation", req(json!([])))),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    let id = v["result"]["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(
            2,
            "wait_for_simulation",
            json!({"id": id, "until": "done", "timeout_secs": 60}),
        )),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["stop_reason"]["kind"], "decision",
        "{v}"
    );
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(3, "get_results", json!({"id": id}))),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["summary"]["spd"]["pending_decision"],
        first
    );

    // refused: slingshot-exact (before the job), an unknown policy (the job fails)
    let (s, _) = call(
        &app,
        "POST",
        "/v1/simulations",
        Some(json!({"files": files, "config": {"flavour": "slingshot-exact", "max_sim_time": 10, "spd_decisions": []}})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let v = run(&app, req(json!([["no such policy"]]))).await;
    assert_eq!(v["status"], "failed", "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("no scaling policy named no such policy"),
        "{v}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn slingshot_exact_runs() {
    let app = app();
    let v = run(
        &app,
        json!({"model_dir": "x_sl_loadbalancer", "config": {"flavour": "slingshot-exact", "max_sim_time": 50}}),
    )
    .await;
    assert_eq!(v["status"], "completed", "{v}");
    assert_eq!(v["model"]["flavour"], "slingshot-exact");
    assert_eq!(v["stop_reason"]["kind"], "simulator");
    assert_eq!(v["progress"]["sim_time"], 50.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn token_and_origin_guard() {
    let app = app_with(HttpConfig {
        token: Some("s3cret".into()),
        ..HttpConfig::default()
    });
    let (s, _) = call_text(&app, "GET", "/health", None, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call_text(&app, "GET", "/v1/info", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call_text(&app, "GET", "/v1/info", None, Some("wrong")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call_text(&app, "GET", "/v1/info", None, Some("s3cret")).await;
    assert_eq!(s, StatusCode::OK);
    let req = Request::builder()
        .uri("/v1/info")
        .header(header::AUTHORIZATION, "Bearer s3cret")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

/// A wait longer than `Duration` can hold is capped, not a panic (which aborts a release build).
#[tokio::test(flavor = "multi_thread")]
async fn oversized_waits_are_capped() {
    let app = app();
    let v = run(
        &app,
        json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": 20}}),
    )
    .await;
    let id = v["id"].as_str().unwrap();
    for wait in ["1e20", "1e300", "inf", "NaN", "-5"] {
        let (s, v) = call(
            &app,
            "GET",
            &format!("/v1/simulations/{id}?wait={wait}&until=done"),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "wait={wait}: {v}");
        assert_eq!(v["status"], "completed");
    }
}

/// The guard against browsers and DNS rebinding: `Origin: null`, a foreign `Host` and `/mcp`
/// bodies that are not JSON are refused; `--allow-host` names more hosts.
#[tokio::test(flavor = "multi_thread")]
async fn host_origin_and_content_type_guard() {
    let app = app_with(HttpConfig {
        allowed_hosts: vec!["sim.lab".into()],
        ..HttpConfig::default()
    });
    let get = |host: &str, origin: Option<&str>| {
        let mut b = Request::builder()
            .uri("/v1/models")
            .header(header::HOST, host);
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        b.body(Body::empty()).unwrap()
    };
    for (host, origin, want) in [
        ("127.0.0.1:8080", None, StatusCode::OK),
        (
            "localhost:8080",
            Some("http://localhost:8080"),
            StatusCode::OK,
        ),
        ("sim.lab:8080", Some("http://sim.lab:8080"), StatusCode::OK),
        ("127.0.0.1:8080", Some("null"), StatusCode::FORBIDDEN),
        ("evil.example:8080", None, StatusCode::FORBIDDEN),
        (
            "127.0.0.1:8080",
            Some("https://evil.example"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let (s, body) = send(&app, get(host, origin)).await;
        assert_eq!(s, want, "Host {host}, Origin {origin:?}: {body}");
    }
    // a page's text/plain POST needs no preflight: /mcp takes JSON only
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    for (content_type, want) in [
        ("text/plain", StatusCode::UNSUPPORTED_MEDIA_TYPE),
        ("application/json; charset=utf-8", StatusCode::OK),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(ping.clone()))
            .unwrap();
        assert_eq!(send(&app, req).await.0, want, "{content_type}");
    }
    // the health check stays open, and --allow-any-origin turns the guard off
    let req = Request::builder()
        .uri("/health")
        .header(header::HOST, "evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::OK);
    let open = app_with(HttpConfig {
        allow_any_origin: true,
        ..HttpConfig::default()
    });
    assert_eq!(
        send(&open, get("evil.example", Some("null"))).await.0,
        StatusCode::OK
    );
}

/// Beyond the running and the queued simulations a submit is refused (HTTP 429, an MCP tool
/// error); a cancelled queued simulation leaves the queue at once and frees its place.
#[tokio::test(flavor = "multi_thread")]
async fn the_queue_is_bounded() {
    let app = app_of(
        ServerConfig {
            allow_dirs: vec![repo().join("corpus")],
            max_concurrent: 1,
            max_queued: 1,
            ..ServerConfig::default()
        },
        HttpConfig::default(),
    );
    let long = json!({"model_dir": "x_sl_mediastore", "config": {"max_measurements": -1},
                      "termination": {"timeout_secs": 60}});
    let (s, running) = call(&app, "POST", "/v1/simulations", Some(long.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{running}");
    let (s, queued) = call(&app, "POST", "/v1/simulations", Some(long.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{queued}");
    assert_eq!(queued["status"], "queued");
    let (s, v) = call(&app, "POST", "/v1/simulations", Some(long.clone())).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(1, "start_simulation", long.clone())),
    )
    .await;
    assert_eq!(v["result"]["isError"], true, "{v}");
    assert!(
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("try again")
    );

    let queued_id = queued["id"].as_str().unwrap();
    let (s, v) = call(
        &app,
        "POST",
        &format!("/v1/simulations/{queued_id}/cancel"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "cancelled", "{v}");
    let (s, v) = call(&app, "POST", "/v1/simulations", Some(long)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    // cancelling the running one starts the next
    let running_id = running["id"].as_str().unwrap();
    call(
        &app,
        "POST",
        &format!("/v1/simulations/{running_id}/cancel"),
        None,
    )
    .await;
    let next = v["id"].as_str().unwrap();
    let (_, v) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{next}?wait=30&after_version=3"),
        None,
    )
    .await;
    assert_ne!(v["status"], "queued", "{v}");
    call(
        &app,
        "POST",
        &format!("/v1/simulations/{next}/cancel"),
        None,
    )
    .await;
    let (_, v) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{running_id}?wait=30&until=done"),
        None,
    )
    .await;
    assert_eq!(v["status"], "cancelled", "{v}");
    assert_eq!(v["stop_reason"]["detail"], "cancelled by a client", "{v}");
}

/// A model name below several allowed directories needs `root`; with it, the model of that
/// directory runs (HTTP and MCP).
#[tokio::test(flavor = "multi_thread")]
async fn root_selects_among_models_of_the_same_name() {
    let base = std::env::temp_dir().join(format!("simoxide-server-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let model = repo().join("corpus/x_sl_mediastore");
    for (root, seed) in [("a", 11), ("b", 22)] {
        let dir = base.join(root).join("m");
        std::fs::create_dir_all(&dir).unwrap();
        for e in std::fs::read_dir(&model).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() && p.file_name().unwrap() != "run.json" {
                std::fs::copy(&p, dir.join(p.file_name().unwrap())).unwrap();
            }
        }
        std::fs::write(
            dir.join("run.json"),
            format!(r#"{{"seed": {seed}, "max_measurements": 5}}"#),
        )
        .unwrap();
    }
    let app = app_of(
        ServerConfig {
            allow_dirs: vec![base.join("a"), base.join("b")],
            ..ServerConfig::default()
        },
        HttpConfig::default(),
    );
    let (_, v) = call(&app, "GET", "/v1/models", None).await;
    let roots: Vec<&str> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["model_dir"] == "m")
        .map(|m| m["root"].as_str().unwrap())
        .collect();
    assert_eq!(roots.len(), 2, "{v}");
    let (s, v) = call(
        &app,
        "POST",
        "/v1/simulations",
        Some(json!({"model_dir": "m"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("root"), "{v}");
    let v = run(&app, json!({"model_dir": "m", "root": roots[1]})).await;
    assert_eq!(v["status"], "completed", "{v}");
    assert_eq!(v["model"]["seed"], 22, "{v}");
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(
            1,
            "start_simulation",
            json!({"model_dir": "m", "root": roots[0]}),
        )),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    let id = v["result"]["structuredContent"]["id"].as_str().unwrap();
    let (_, v) = call(
        &app,
        "GET",
        &format!("/v1/simulations/{id}?wait=60&until=done"),
        None,
    )
    .await;
    assert_eq!(v["model"]["seed"], 11, "{v}");
    let _ = std::fs::remove_dir_all(&base);
}

/// A passed `max_sim_time` still counts inside `all` (it used to read false again once the
/// engine stopped reporting the next event's time, so the run went on).
#[tokio::test(flavor = "multi_thread")]
async fn sim_time_inside_all_ends_the_run() {
    let app = app();
    for flavour in ["slingshot-exact", "simulizar"] {
        let v = run(
            &app,
            json!({"model_dir": "x_sl_loadbalancer",
                   "config": {"flavour": flavour, "max_sim_time": 100_000_000, "max_measurements": -1},
                   "termination": {"combine": "all", "max_sim_time": 5, "timeout_secs": 0.2},
                   "progress": {"interval_ms": 50}}),
        )
        .await;
        assert_eq!(v["stop_reason"]["kind"], "termination", "{flavour}: {v}");
        let d = v["stop_reason"]["detail"].as_str().unwrap();
        assert!(
            d.contains("max_sim_time") && d.contains("timeout_secs"),
            "{flavour}: {d}"
        );
    }
}

/// The server's resource limits reach the slingshot-exact engine.
#[tokio::test(flavor = "multi_thread")]
async fn slingshot_exact_enforces_resource_limits() {
    let app = app();
    let v = run(
        &app,
        json!({"model_dir": "x_sl_loadbalancer",
               "config": {"flavour": "slingshot-exact", "max_sim_time": 100_000, "max_steps": 1000}}),
    )
    .await;
    assert_eq!(v["status"], "failed", "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("Limits::max_steps"),
        "{v}"
    );
    assert_eq!(v["results_available"], true, "{v}");
}

/// `config.interval_summary` (`docs/guide/server.md`, "Interval summaries"): the run's single
/// interval `0..tuples`; per selected series the statistics of its rows in
/// `measurements.csv`, through HTTP and MCP; an unknown statistic is refused.
#[tokio::test(flavor = "multi_thread")]
async fn interval_summary_of_the_run() {
    let app = app();
    let files = files_of(&repo().join("corpus/x_sl_mediastore"));
    let req = json!({"files": files, "config": {"seed": 3, "max_measurements": 200,
        "interval_summary": {"series": [{"metric": "Response Time"}, {"metric": "State of Active"}],
                             "stats": ["mean", "max", "last", "initial", "time_weighted"]}}});
    let v = run(&app, req.clone()).await;
    assert_eq!(v["status"], "completed", "{v}");
    let id = v["id"].as_str().unwrap();
    let res = results(&app, id, "").await;
    let sum = &res["summary"];
    let doc = &sum["interval_summary"];
    assert_eq!(doc["format"], "simoxide-interval-summary");
    assert_eq!(doc["version"], 1);
    assert_eq!(
        doc["interval"]["tuples"],
        json!({"from": 0, "to": sum["tuples"]})
    );
    assert_eq!(doc["interval"]["time"]["from"], 0.0);
    assert_eq!(doc["interval"]["time"]["to"], sum["sim_time"]);
    assert_eq!(
        doc["stats"],
        json!(["count", "mean", "max", "last", "initial", "time_weighted"])
    );
    let (_, csv) = call_text(
        &app,
        "GET",
        &format!("/v1/simulations/{id}/measurements.csv"),
        None,
        None,
    )
    .await;
    // the rows of each series, in file order
    let mut rows: std::collections::BTreeMap<(String, String), Vec<(f64, f64)>> =
        Default::default();
    for l in csv.lines().skip(1) {
        let f: Vec<&str> = l.rsplitn(4, ',').collect();
        rows.entry((f[3].to_string(), f[2].to_string()))
            .or_default()
            .push((f[1].parse().unwrap(), f[0].parse().unwrap()));
    }
    let series = doc["series"].as_array().unwrap();
    let keys: Vec<(String, String)> = series
        .iter()
        .map(|s| {
            (
                s["measuring_point"].as_str().unwrap().to_string(),
                s["metric"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    // every selected series with rows, and only selected ones (also those without rows)
    for k in rows.keys() {
        let selected = k.1.contains("Response Time") || k.1.contains("State of Active");
        assert_eq!(keys.contains(k), selected, "{k:?}");
    }
    assert!(
        keys.iter()
            .all(|k| k.1.contains("Response Time") || k.1.contains("State of Active"))
    );
    assert!(series.iter().any(|s| s["unit"] == "s"));
    assert!(series.iter().any(|s| s["unit"] == "1"));
    for (s, key) in series.iter().zip(&keys) {
        let Some(r) = rows.get(key) else {
            // a series without tuples: explicit nulls
            assert_eq!(s["count"], 0, "{s}");
            assert_eq!(
                (&s["mean"], &s["max"], &s["last"]),
                (&json!(null), &json!(null), &json!(null))
            );
            assert_eq!(s["time_weighted"], json!(null), "{s}");
            continue;
        };
        assert_eq!(s["count"], r.len(), "{s}");
        let total: f64 = r.iter().map(|x| x.1).sum();
        assert_eq!(s["mean"].as_f64().unwrap(), total / r.len() as f64, "{s}");
        let max = r.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(s["max"].as_f64().unwrap(), max);
        let last = r.last().unwrap();
        assert_eq!(s["last"], json!({"time": last.0, "value": last.1}));
        assert_eq!(s["initial"], json!({"status": "none"}));
        assert!(s.get("sum").is_none() && s.get("min").is_none());
        // the step function from the first row to the end of the run
        let tw = &s["time_weighted"];
        assert_eq!(tw["from"].as_f64().unwrap(), r[0].0);
        let end = sum["sim_time"].as_f64().unwrap();
        let mut integral = 0.0;
        for (i, &(t, v)) in r.iter().enumerate() {
            let next = r.get(i + 1).map_or(end, |x| x.0);
            integral += v * (next - t);
        }
        let got = tw["integral"].as_f64().unwrap();
        assert!(
            (got - integral).abs() <= 1e-9 * integral.abs().max(1.0),
            "{tw} vs {integral}"
        );
    }

    // MCP: the same option of start_simulation, the same document
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(1, "start_simulation", req.clone())),
    )
    .await;
    assert_eq!(v["result"]["isError"], false, "{v}");
    let mid = v["result"]["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(
            2,
            "wait_for_simulation",
            json!({"id": mid, "until": "done", "timeout_secs": 60}),
        )),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["status"], "completed",
        "{v}"
    );
    let (_, v) = call(
        &app,
        "POST",
        "/mcp",
        Some(tool_call(3, "get_results", json!({"id": mid}))),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["summary"]["interval_summary"],
        *doc
    );

    // refused: an unknown statistic
    let mut bad = req;
    bad["config"]["interval_summary"]["stats"] = json!(["median"]);
    let (s, v) = call(&app, "POST", "/v1/simulations", Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("median"), "{v}");
}
