//! The HTTP API (axum): a REST interface, a server-sent-events stream of live progress and logs,
//! and MCP's streamable HTTP transport at `/mcp`.
//!
//! | Method and path | Purpose |
//! |---|---|
//! | `GET /health` | liveness (no authentication) |
//! | `GET /v1/info` | version, features, limits |
//! | `GET /v1/models` | model directories below `--allow-dir` |
//! | `POST /v1/simulations` | start a simulation ([`SimulationRequest`]); `201` with its state |
//! | `GET /v1/simulations` | list simulations |
//! | `GET /v1/simulations/{id}` | state and live progress; `?wait=S&until=change\|done&after_version=V&logs=N` long-polls |
//! | `GET /v1/simulations/{id}/logs` | log entries `?since=SEQ&limit=N&level=debug\|info\|warn\|error` |
//! | `GET /v1/simulations/{id}/events` | server-sent events: `status`, `progress`, `log`, `done` |
//! | `GET /v1/simulations/{id}/results` | final results `?rows=true&max_rows=N&measuring_point=..&metric=..` |
//! | `GET /v1/simulations/{id}/measurements.csv` | `measurements.csv` `?measuring_point=..&metric=..` |
//! | `GET /v1/simulations/{id}/runtime-metadata` | model file and runtime file (`config.runtime_metadata`) |
//! | `POST /v1/simulations/{id}/cancel` | stop; results so far stay readable |
//! | `DELETE /v1/simulations/{id}` | cancel and forget |
//! | `POST /mcp` | MCP (JSON-RPC over streamable HTTP, JSON responses) |
//!
//! Errors are JSON objects `{"error": "..."}` with a 4xx status. With a token
//! (`--token`/`SIMOXIDE_SERVER_TOKEN`) every route but `/health` needs
//! `Authorization: Bearer <token>`. Unless the server allows any origin, every route but
//! `/health` refuses requests whose `Host` header or `Origin` header names another host than
//! `localhost`, `127.0.0.1`, `[::1]` or one of the configured host names (MCP's guard against
//! DNS rebinding; the opaque origin `null` of sandboxed pages is refused too), and `/mcp` takes
//! only `application/json` bodies (a page cannot send those without a preflight).

use crate::api::{
    DEFAULT_RECENT_LOGS, MAX_LOG_PAGE, MAX_WAIT, ResultsQuery, Service, Until, to_json,
    without_series,
};
use crate::job::{ApiError, JobView, MIB};
use crate::mcp::{Mcp, RPC_PARSE_ERROR, rpc_error};
use crate::report::{JobStatus, Level};
use crate::request::{SeriesSelector, SimulationRequest};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

/// Default of [`HttpConfig::max_body_bytes`].
pub const DEFAULT_MAX_BODY_BYTES: usize = 64 * MIB;
/// Log entries of one `GET .../logs` response, unless the client asks otherwise.
const DEFAULT_LOG_PAGE: usize = 1000;
/// An event stream looks for a change at least this often, and is kept alive this often.
const EVENT_STREAM_PERIOD: Duration = Duration::from_secs(15);
/// Host names a local server answers to, besides [`HttpConfig::allowed_hosts`].
const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
/// The media type `/mcp` requires.
const JSON_MEDIA_TYPE: &str = "application/json";

/// Settings of the HTTP layer.
#[derive(Clone, Debug)]
pub struct HttpConfig {
    /// Bearer token every route but `/health` requires (`None`: no authentication).
    pub token: Option<String>,
    /// Largest request body in bytes.
    pub max_body_bytes: usize,
    /// Accept requests with any `Host` and `Origin` header (default: only local hosts and
    /// [`HttpConfig::allowed_hosts`]).
    pub allow_any_origin: bool,
    /// Host names besides the local ones that `Host` and `Origin` headers may name (e.g. the
    /// server's name on a network), without a port.
    pub allowed_hosts: Vec<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig {
            token: None,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            allow_any_origin: false,
            allowed_hosts: Vec::new(),
        }
    }
}

/// What every request handler has access to.
#[derive(Clone)]
struct AppState {
    svc: Arc<Service>,
    mcp: Arc<Mcp>,
    cfg: Arc<HttpConfig>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// The router with every route (see the module documentation).
pub fn router(svc: Arc<Service>, cfg: HttpConfig) -> Router {
    let state = AppState {
        mcp: Arc::new(Mcp { svc: svc.clone() }),
        svc,
        cfg: Arc::new(cfg.clone()),
    };
    let api = Router::new()
        .route("/v1/info", get(info))
        .route("/v1/models", get(models))
        .route("/v1/simulations", post(start).get(list))
        .route("/v1/simulations/{id}", get(status).delete(delete))
        .route("/v1/simulations/{id}/logs", get(logs))
        .route("/v1/simulations/{id}/events", get(events))
        .route("/v1/simulations/{id}/results", get(results))
        .route("/v1/simulations/{id}/measurements.csv", get(csv))
        .route(
            "/v1/simulations/{id}/runtime-metadata",
            get(runtime_metadata),
        )
        .route("/v1/simulations/{id}/cancel", post(cancel))
        .route("/mcp", post(mcp_post).get(mcp_get).delete(mcp_get))
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard));
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(api)
        .fallback(|| async {
            ApiError {
                status: StatusCode::NOT_FOUND,
                message: "no such route (see GET /v1/info and docs/guide/server.md)".into(),
            }
        })
        .layer(DefaultBodyLimit::max(cfg.max_body_bytes))
        .with_state(state)
}

/// Authentication and the host and origin checks.
async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if !st.cfg.allow_any_origin {
        let forbidden = |message: &str| {
            ApiError {
                status: StatusCode::FORBIDDEN,
                message: message.into(),
            }
            .into_response()
        };
        // a missing `Host` is no browser's request (browsers always send it)
        if let Some(h) = req.headers().get(header::HOST)
            && !allowed_host(h, &st.cfg.allowed_hosts)
        {
            return forbidden(
                "host not allowed (start the server with --allow-host NAME or --allow-any-origin)",
            );
        }
        if let Some(o) = req.headers().get(header::ORIGIN)
            && !allowed_origin(o, &st.cfg.allowed_hosts)
        {
            return forbidden(
                "origin not allowed (start the server with --allow-host NAME or --allow-any-origin)",
            );
        }
    }
    if let Some(t) = &st.cfg.token {
        let ok = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|got| constant_time_eq(got.as_bytes(), t.as_bytes()));
        if !ok {
            let mut r = ApiError {
                status: StatusCode::UNAUTHORIZED,
                message: "missing or wrong bearer token".into(),
            }
            .into_response();
            r.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            return r;
        }
    }
    next.run(req).await
}

/// The host of an authority (`host`, `host:port`, `[v6]:port`), without the port.
fn host_of_authority(authority: &str) -> &str {
    match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(v6),
        None => authority.split(':').next().unwrap_or(authority),
    }
}

/// Whether `host` is a local host name or one of `allowed`.
fn is_allowed_host(host: &str, allowed: &[String]) -> bool {
    LOCAL_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h))
        || allowed.iter().any(|h| host.eq_ignore_ascii_case(h))
}

/// Whether a `Host` header names this machine or an allowed host.
fn allowed_host(h: &HeaderValue, allowed: &[String]) -> bool {
    h.to_str()
        .is_ok_and(|s| is_allowed_host(host_of_authority(s), allowed))
}

/// Whether an `Origin` header (`scheme://host[:port]`) names this machine or an allowed host.
/// The opaque origin `null` (sandboxed frames, `file:` pages) is not allowed.
fn allowed_origin(o: &HeaderValue, allowed: &[String]) -> bool {
    let Ok(s) = o.to_str() else {
        return false;
    };
    let Some((_, rest)) = s.split_once("://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    is_allowed_host(host_of_authority(authority), allowed)
}

/// Whether a request's `Content-Type` is JSON (`application/json`, parameters allowed).
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|t| t.trim().eq_ignore_ascii_case(JSON_MEDIA_TYPE))
}

/// Compares without an early exit, so that the time does not tell how much of a token is right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Runs a blocking call off the async workers.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("internal error: {e}"),
    })?
}

async fn info(State(st): State<AppState>) -> Json<Value> {
    let mut v = st.svc.info();
    v["endpoints"] = json!({
        "rest": "/v1/simulations",
        "events": "/v1/simulations/{id}/events",
        "mcp": "/mcp",
    });
    Json(v)
}

async fn models(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "models": st.svc.models()? })))
}

async fn start(
    State(st): State<AppState>,
    body: Result<Json<SimulationRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Response> {
    let Json(req) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let svc = st.svc.clone();
    let v = blocking(move || svc.start(req)).await?;
    let loc = format!("/v1/simulations/{}", v.id);
    let mut r = (StatusCode::CREATED, Json(v)).into_response();
    if let Ok(h) = HeaderValue::from_str(&loc) {
        r.headers_mut().insert(header::LOCATION, h);
    }
    Ok(r)
}

async fn list(State(st): State<AppState>) -> Json<Value> {
    Json(json!({ "simulations": st.svc.list() }))
}

/// The query of `GET /v1/simulations/{id}`.
#[derive(Deserialize)]
struct StatusQuery {
    /// Long-poll for at most this many seconds.
    wait: Option<f64>,
    /// What to long-poll for.
    until: Option<Until>,
    /// Long-poll for a version above this one.
    after_version: Option<u64>,
    /// Newest log entries to include (default [`DEFAULT_RECENT_LOGS`]).
    logs: Option<usize>,
    /// `false`: leave out the per-series statistics.
    series: Option<bool>,
}

async fn status(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StatusQuery>,
) -> ApiResult<Json<Value>> {
    let recent_logs = q.logs.unwrap_or(DEFAULT_RECENT_LOGS);
    let v = match q.wait {
        Some(w) if w > 0.0 && w.is_finite() => {
            // capped before the conversion: `Duration::from_secs_f64` panics beyond 2^64 s
            let timeout = Duration::from_secs_f64(w.min(MAX_WAIT.as_secs_f64()));
            let until = q.until.unwrap_or_default();
            st.svc
                .wait(&id, q.after_version, until, timeout, recent_logs)
                .await?
        }
        _ => st.svc.status(&id, recent_logs)?,
    };
    let v = to_json(&v);
    Ok(Json(if q.series == Some(false) {
        without_series(v)
    } else {
        v
    }))
}

/// The query of `GET /v1/simulations/{id}/logs`.
#[derive(Deserialize)]
struct LogsQuery {
    /// Entries after this sequence number (default 0: from the start).
    since: Option<u64>,
    /// At most this many entries (default [`DEFAULT_LOG_PAGE`]).
    limit: Option<usize>,
    /// Minimum level (default info).
    level: Option<String>,
}

/// The query parameter `level`.
fn level_parameter(s: Option<&str>) -> ApiResult<Level> {
    Level::from_query(s).map_err(|e| ApiError::bad_request(format!("level: {e}")))
}

async fn logs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Json<Value>> {
    let min_level = level_parameter(q.level.as_deref())?;
    let page = st.svc.logs(
        &id,
        q.since.unwrap_or(0),
        q.limit.unwrap_or(DEFAULT_LOG_PAGE),
        min_level,
    )?;
    Ok(Json(to_json(&page)))
}

/// The query of `GET /v1/simulations/{id}/events`.
#[derive(Deserialize)]
struct EventsQuery {
    /// Minimum level of the forwarded log entries (default info).
    level: Option<String>,
    /// Forward log entries (default true).
    logs: Option<bool>,
    /// Log entries after this sequence number (default 0: the whole log).
    since: Option<u64>,
}

/// State of one event stream.
struct Feed {
    svc: Arc<Service>,
    /// The simulation it follows.
    id: String,
    /// The version seen last.
    version: u64,
    /// The log entry sent last.
    log_seq: u64,
    /// The progress sent last.
    progress: Option<Arc<crate::report::Progress>>,
    /// The status sent last.
    status: Option<JobStatus>,
    /// Minimum level of the forwarded log entries.
    level: Level,
    /// Forward log entries.
    logs: bool,
    /// The last event was sent.
    done: bool,
}

/// A server-sent event with a JSON payload.
fn event(name: &str, v: &impl serde::Serialize) -> Event {
    Event::default()
        .event(name)
        .json_data(v)
        .unwrap_or_else(|_| Event::default().event(name).data("{}"))
}

impl Feed {
    /// Waits for the next change (at most [`EVENT_STREAM_PERIOD`]) and returns the events it
    /// makes.
    async fn next(mut self) -> Option<(Vec<Result<Event, Infallible>>, Feed)> {
        if self.done {
            return None;
        }
        let mut out = Vec::new();
        let job = match self.svc.jobs.get(&self.id) {
            Ok(job) => job,
            Err(e) => {
                self.done = true;
                out.push(Ok(event("error", &json!({"error": e.message}))));
                return Some((out, self));
            }
        };
        if self.status.is_some() {
            job.wait(self.version, false, EVENT_STREAM_PERIOD).await;
        }
        let v = job.view(0, true);
        self.version = v.version;
        if self.logs {
            if let Ok(page) = self
                .svc
                .logs(&self.id, self.log_seq, MAX_LOG_PAGE, self.level)
            {
                for e in &page.entries {
                    out.push(Ok(event("log", e)));
                }
                self.log_seq = self.log_seq.max(page.last_seq);
            }
        } else {
            self.log_seq = v.last_log_seq;
        }
        if self.status != Some(v.status) {
            self.status = Some(v.status);
            let mut s = v.clone();
            s.progress = None;
            out.push(Ok(event("status", &s)));
        }
        let changed = match (&self.progress, &v.progress) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, Some(_)) => true,
            _ => false,
        };
        if changed && let Some(p) = &v.progress {
            out.push(Ok(event("progress", p.as_ref())));
            self.progress = v.progress.clone();
        }
        if v.status.is_terminal() {
            self.done = true;
            out.push(Ok(event(
                "done",
                &json!({
                    "id": v.id,
                    "status": v.status,
                    "stop_reason": v.stop_reason,
                    "error": v.error,
                    "results_available": v.results_available,
                }),
            )));
        }
        Some((out, self))
    }
}

async fn events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    st.svc.jobs.get(&id)?;
    let feed = Feed {
        svc: st.svc.clone(),
        id,
        version: 0,
        log_seq: q.since.unwrap_or(0),
        progress: None,
        status: None,
        level: level_parameter(q.level.as_deref())?,
        logs: q.logs.unwrap_or(true),
        done: false,
    };
    let s = stream::unfold(feed, Feed::next).flat_map(stream::iter);
    Ok(Sse::new(s).keep_alive(KeepAlive::new().interval(EVENT_STREAM_PERIOD)))
}

/// The query of `GET /v1/simulations/{id}/results`.
#[derive(Deserialize)]
struct ResultsParams {
    /// Include the raw rows.
    rows: Option<bool>,
    /// Rows per series (see [`ResultsQuery::max_rows_per_series`]).
    max_rows: Option<usize>,
    /// Only series whose measuring point contains this.
    measuring_point: Option<String>,
    /// Only series whose metric contains this.
    metric: Option<String>,
}

/// The series selection of the query parameters (no parameter: all series).
fn selectors(mp: Option<String>, metric: Option<String>) -> Vec<SeriesSelector> {
    if mp.is_none() && metric.is_none() {
        return Vec::new();
    }
    vec![SeriesSelector {
        measuring_point: mp,
        metric,
    }]
}

async fn results(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ResultsParams>,
) -> ApiResult<Json<Value>> {
    let rq = ResultsQuery {
        include_rows: q.rows.unwrap_or(false),
        series: selectors(q.measuring_point, q.metric),
        max_rows_per_series: q.max_rows,
    };
    Ok(Json(st.svc.results(&id, &rq)?))
}

/// The query of `GET /v1/simulations/{id}/measurements.csv`.
#[derive(Deserialize)]
struct CsvParams {
    /// Only series whose measuring point contains this.
    measuring_point: Option<String>,
    /// Only series whose metric contains this.
    metric: Option<String>,
}

async fn csv(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<CsvParams>,
) -> ApiResult<Response> {
    let svc = st.svc.clone();
    let sel = selectors(q.measuring_point, q.metric);
    let body = blocking(move || svc.csv(&id, &sel)).await?;
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    Ok((h, body).into_response())
}

async fn runtime_metadata(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let svc = st.svc.clone();
    Ok(Json(blocking(move || svc.runtime_metadata(&id)).await?))
}

async fn cancel(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    Ok(Json(st.svc.cancel(&id)?))
}

async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    st.svc.jobs.delete(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// MCP streamable HTTP: one JSON-RPC message or a batch; notifications and responses are
/// accepted with `202`. The body must be `application/json` (`415` otherwise).
async fn mcp_post(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !is_json(&headers) {
        return ApiError {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            message: format!("POST /mcp needs Content-Type: {JSON_MEDIA_TYPE}"),
        }
        .into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let error = rpc_error(Value::Null, RPC_PARSE_ERROR, format!("parse error: {e}"));
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };
    match st.mcp.handle_message(msg).await {
        Some(r) => Json(r).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// The optional server-to-client SSE stream of MCP is not offered.
async fn mcp_get() -> Response {
    let mut r = (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({"error": "this MCP endpoint answers POST requests only (no server-initiated stream, no sessions)"})),
    )
        .into_response();
    r.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    r
}

/// Serves `router` on `addr` until ctrl-c; then cancels the running simulations.
pub async fn serve(
    svc: Arc<Service>,
    cfg: HttpConfig,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    let jobs = svc.jobs.clone();
    let app = router(svc, cfg);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("simoxide-server: shutting down, cancelling running simulations");
            jobs.cancel_all();
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        let allowed = vec!["sim.lab".to_string()];
        for ok in [
            "http://localhost:3000",
            "http://127.0.0.1",
            "https://[::1]:8080",
            "http://LOCALHOST",
            "https://sim.lab:8443",
        ] {
            assert!(
                allowed_origin(&HeaderValue::from_static(ok), &allowed),
                "{ok}"
            );
        }
        for bad in [
            "https://evil.example",
            "http://localhost.evil.example",
            "null",
            "localhost",
        ] {
            assert!(
                !allowed_origin(&HeaderValue::from_static(bad), &allowed),
                "{bad}"
            );
        }
    }

    #[test]
    fn hosts() {
        for ok in ["localhost:8080", "127.0.0.1", "[::1]:8080", "sim.lab:80"] {
            assert!(
                allowed_host(&HeaderValue::from_static(ok), &["sim.lab".into()]),
                "{ok}"
            );
        }
        for bad in ["evil.example:8080", "127.0.0.1.evil.example", "sim.lab"] {
            assert!(!allowed_host(&HeaderValue::from_static(bad), &[]), "{bad}");
        }
    }

    #[test]
    fn json_content_type() {
        let mut h = HeaderMap::new();
        assert!(!is_json(&h));
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!is_json(&h));
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("Application/JSON; charset=utf-8"),
        );
        assert!(is_json(&h));
    }
}
