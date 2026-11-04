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
    let server = ServerConfig {
        allow_dirs: vec![repo().join("corpus"), repo().join("corpus-spd")],
        max_concurrent: 4,
        ..ServerConfig::default()
    };
    router(
        Arc::new(Service {
            jobs: JobManager::new(server),
        }),
        cfg,
    )
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
