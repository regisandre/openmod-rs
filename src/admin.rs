//! The admin web interface: tools, workflows, API keys and a test bench, on
//! its own listener. Every API route but the sign-in needs a session, and
//! every write must come from the interface itself.

use crate::config::{Config, RULE_CATEGORIES};
use crate::download::{self, Downloads, Needed, Progress};
use crate::edit::{self, Edit};
use crate::engine::{Engine, Report, ToolStatus};
use crate::keys::{self, State};
use crate::server::{ApiError, Shared, TextBody, WorkflowQuery, run_check};
use crate::tools::{self, Kind, Tool};
use crate::workflow::Threshold;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State as Extract};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const INDEX: &str = include_str!("../ui/index.html");
const CSS: &str = include_str!("../ui/app.css");
const JS: &str = include_str!("../ui/app.js");

pub const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const COOKIE: &str = "openmod_admin";
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; \
                   connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Open admin sessions, in memory: a restart signs everyone out. Each one
/// remembers the admin token it was opened with, so rotating the token
/// closes them all.
pub struct Sessions {
    ttl: Duration,
    open: Mutex<HashMap<[u8; 32], Session>>,
}

/// When a session expires, and the digest of the admin token it was opened with.
type Session = (Instant, [u8; 32]);

impl Sessions {
    pub fn new(ttl: Duration) -> Sessions {
        Sessions {
            ttl,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Opens a session for the admin token with digest `admin` and returns
    /// its id. Only the digest of the id is kept.
    pub fn create(&self, admin: [u8; 32]) -> String {
        let id = keys::generate("");
        let now = Instant::now();
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.retain(|_, (expires, _)| *expires > now);
        open.insert(keys::digest(&id), (now + self.ttl, admin));
        id
    }

    /// True for a live session opened with the current admin token.
    pub fn check(&self, id: &str, admin: Option<[u8; 32]>) -> bool {
        let Some(admin) = admin else { return false };
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.get(&keys::digest(id))
            .is_some_and(|(expires, a)| *expires > Instant::now() && *a == admin)
    }

    pub fn remove(&self, id: &str) {
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&keys::digest(id));
    }
}

/// A write must come from the interface itself. The `x-openmod-admin`
/// header cannot be sent cross-site without a CORS preflight, which this
/// server never grants; a browser also says where a request comes from
/// (`Sec-Fetch-Site`, `Origin`), and that must be this very site. A client
/// that sends no browser headers (curl with a session cookie) only needs
/// the header.
pub fn same_origin_write(method: &Method, headers: &HeaderMap) -> bool {
    if matches!(*method, Method::GET | Method::HEAD) {
        return true;
    }
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if get("x-openmod-admin").is_none() {
        return false;
    }
    if get("sec-fetch-site").is_some_and(|site| site != "same-origin") {
        return false;
    }
    if let Some(origin) = get("origin") {
        let authority = origin
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let hosts = [get("host"), get("x-forwarded-host")];
        if authority.is_empty() || !hosts.iter().flatten().any(|h| *h == authority) {
            return false;
        }
    }
    true
}

/// The session cookie: out of reach of scripts (`HttpOnly`) and never sent
/// by another site (`SameSite=Strict`). `Secure` behind an HTTPS proxy.
pub fn session_cookie(id: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{COOKIE}={id}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{secure}",
        SESSION_TTL.as_secs()
    )
}

fn session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| {
            pair.trim()
                .strip_prefix(&format!("{COOKIE}="))
                .map(str::to_string)
        })
        .next()
}

fn behind_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https")
}

fn signed_in(app: &Shared, headers: &HeaderMap) -> bool {
    let Some(id) = session_id(headers) else {
        return false;
    };
    let admin = State::load(&app.state_path)
        .ok()
        .and_then(|s| s.admin_digest());
    app.sessions.check(&id, admin)
}

async fn guard(Extract(app): Extract<Shared>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let open = matches!(
        path,
        "/" | "/app.css" | "/app.js" | "/api/session" | "/api/login"
    );
    let mut response = if !same_origin_write(request.method(), request.headers()) {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "cross-site request refused",
        )
        .into_response()
    } else if !open && !signed_in(&app, request.headers()) {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "sign in to the admin interface first",
        )
        .into_response()
    } else {
        next.run(request).await
    };
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

/// The embedded file, or with `OPENMOD_UI_DIR` set (to work on the
/// interface without rebuilding), the file of that name in that directory.
fn asset(name: &str, embedded: &'static str) -> String {
    match std::env::var_os("OPENMOD_UI_DIR") {
        Some(dir) => {
            std::fs::read_to_string(std::path::Path::new(&dir).join(name)).unwrap_or_default()
        }
        None => embedded.to_string(),
    }
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        asset("index.html", INDEX),
    )
}

async fn css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        asset("app.css", CSS),
    )
}

async fn js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        asset("app.js", JS),
    )
}

async fn session(Extract(app): Extract<Shared>, headers: HeaderMap) -> Json<Value> {
    Json(json!({ "authenticated": signed_in(&app, &headers) }))
}

#[derive(Deserialize)]
struct Login {
    token: String,
}

async fn login(
    Extract(app): Extract<Shared>,
    headers: HeaderMap,
    Json(body): Json<Login>,
) -> Result<Response, ApiError> {
    let state = State::load(&app.state_path).map_err(ApiError::internal)?;
    let admin = state.admin_digest().filter(|_| state.is_admin(&body.token));
    let Some(admin) = admin else {
        // Slows down guessing; the token is 192 random bits anyway.
        tokio::time::sleep(Duration::from_secs(1)).await;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "wrong admin token",
        ));
    };
    let id = app.sessions.create(admin);
    let cookie = session_cookie(&id, behind_https(&headers));
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "authenticated": true })),
    )
        .into_response())
}

async fn logout(Extract(app): Extract<Shared>, headers: HeaderMap) -> Response {
    if let Some(id) = session_id(&headers) {
        app.sessions.remove(&id);
    }
    let cookie = format!("{COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    (
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "authenticated": false })),
    )
        .into_response()
}

fn kind_name(image: bool) -> &'static str {
    if image { "image" } else { "text" }
}

fn status_json(status: Option<&ToolStatus>) -> (&'static str, Option<String>) {
    match status {
        None => ("absent", None),
        Some(ToolStatus::Ready) => ("ready", None),
        Some(ToolStatus::Disabled) => ("disabled", None),
        Some(ToolStatus::Missing { .. }) => ("missing", None),
        Some(ToolStatus::Failed { error }) => ("failed", Some(error.clone())),
    }
}

fn tool_json(engine: &Engine, id: &str, builtin: Option<&Tool>) -> Value {
    let entry = engine.config.models.get(id);
    let (status, error) = status_json(engine.status.get(id));
    let image = match (entry, builtin) {
        (Some(e), _) => e.spec.is_image(),
        (None, Some(t)) => t.kind == Kind::Image,
        (None, None) => false,
    };
    let files: Vec<&std::path::Path> = entry.map(|e| e.spec.files()).unwrap_or_default();
    let missing: Vec<&std::path::Path> = files
        .iter()
        .copied()
        .filter(|p| !tools::present(p))
        .collect();
    let download: u64 = match (entry, builtin) {
        (Some(_), _) => missing
            .iter()
            .filter_map(|p| tools::file_for(p))
            .map(|f| f.size)
            .sum(),
        (None, Some(t)) => t.files.iter().map(|f| f.size).sum(),
        (None, None) => 0,
    };
    let downloadable = missing.iter().all(|p| tools::file_for(p).is_some());
    let categories: Vec<String> = match entry {
        Some(e) => e.spec.categories().into_iter().collect(),
        None => Vec::new(),
    };
    let ram = match entry {
        Some(e) => tools::model_ram_mb(id, &e.spec),
        None => builtin.map(|t| t.ram_mb).unwrap_or(0),
    };
    let role = engine.second_opinion_of(id).map(|_| "second_opinion");
    json!({
        "id": id,
        "name": builtin.map(|t| t.name).unwrap_or(id),
        "kind": kind_name(image),
        "summary": builtin.map(|t| t.summary).unwrap_or("A custom model from the configuration file."),
        "license": builtin.map(|t| t.license),
        "builtin": builtin.is_some(),
        "enabled": entry.is_some_and(|e| e.enabled),
        "status": status,
        "error": error,
        "ram_mb": ram,
        "download_mb": download.div_ceil(1_000_000),
        "missing": missing.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy()).collect::<Vec<_>>(),
        "downloadable": downloadable,
        "categories": categories,
        "role": role,
    })
}

fn categories_json(engine: &Engine, image: bool) -> Vec<Value> {
    let config = &engine.config;
    let available = engine.available(image);
    config
        .known_categories(image)
        .into_iter()
        .map(|id| {
            let label = engine.label(&id);
            let rule = !image && RULE_CATEGORIES.contains(&id.as_str());
            json!({
                "id": id,
                "title": label.text,
                "message": label.message,
                "tools": config.producers(&id, image),
                "rule": rule,
                "available": available.contains(&id),
            })
        })
        .collect()
}

fn rss_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

fn api_url(bind: &str) -> String {
    match bind.parse::<SocketAddr>() {
        Ok(addr) if addr.ip().is_unspecified() => format!("http://127.0.0.1:{}", addr.port()),
        Ok(addr) => format!("http://{addr}"),
        Err(_) => format!("http://{bind}"),
    }
}

/// The workflows with thresholds rounded: `serde_json` widens an `f32` to
/// `f64`, which turns 0.8 into 0.800000011920929.
fn workflows_json(config: &Config) -> Value {
    let round = |v: Option<f32>| v.map(|x| (f64::from(x) * 1000.0).round() / 1000.0);
    let rules = |wf: &crate::workflow::Workflow| -> Value {
        wf.rules
            .iter()
            .map(|(c, t)| {
                let mut o = serde_json::Map::new();
                if let Some(v) = round(t.reject) {
                    o.insert("reject".into(), json!(v));
                }
                if let Some(v) = round(t.review) {
                    o.insert("review".into(), json!(v));
                }
                (c.clone(), Value::Object(o))
            })
            .collect::<serde_json::Map<String, Value>>()
            .into()
    };
    config
        .workflows
        .iter()
        .map(|(name, spec)| {
            (
                name.clone(),
                json!({ "image": rules(&spec.image), "text": rules(&spec.text) }),
            )
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

/// Everything the interface shows, in one document.
fn snapshot(app: &Shared) -> Result<Value, ApiError> {
    let engine = app.engine();
    let config = &engine.config;
    let state = State::load(&app.state_path).map_err(ApiError::internal)?;
    let mut tool_list: Vec<Value> = tools::TOOLS
        .iter()
        .map(|t| tool_json(&engine, t.id, Some(t)))
        .collect();
    for id in config.models.keys().filter(|id| tools::find(id).is_none()) {
        tool_list.push(tool_json(&engine, id, None));
    }
    let keys: Vec<Value> = state
        .api_keys
        .iter()
        .map(|k| json!({ "id": k.id, "name": k.name, "prefix": k.prefix, "created": k.created }))
        .collect();
    let downloads = app
        .downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "setup_done": state.setup_done,
        "api_url": api_url(&config.server.bind),
        "memory": { "estimate_mb": tools::estimate_mb(config), "base_mb": tools::BASE_RAM_MB, "rss_mb": rss_mb() },
        "tools": tool_list,
        "categories": { "image": categories_json(&engine, true), "text": categories_json(&engine, false) },
        "workflows": workflows_json(config),
        "unchecked": engine.unchecked_rules(),
        "keys": keys,
        "external_keys": app.key_count().saturating_sub(state.api_keys.len()),
        "downloads": downloads,
    }))
}

async fn state(Extract(app): Extract<Shared>) -> Result<Json<Value>, ApiError> {
    Ok(Json(snapshot(&app)?))
}

/// Applies a configuration edit, then reloads the engine with it. Edits run
/// one at a time.
async fn apply_edit(app: &Shared, edit: Edit) -> Result<(), ApiError> {
    let _one_at_a_time = app.edits.lock().await;
    let worker = app.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let config = edit::apply_file(&worker.config_path, &edit)?;
        worker.reload(config);
        Ok(())
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_config", format!("{e:#}")))
}

#[derive(Deserialize)]
struct ToolBody {
    enabled: bool,
}

async fn set_tool(
    Extract(app): Extract<Shared>,
    Path(id): Path<String>,
    Json(body): Json<ToolBody>,
) -> Result<Json<Value>, ApiError> {
    apply_edit(
        &app,
        Edit::SetTool {
            id,
            enabled: body.enabled,
        },
    )
    .await?;
    Ok(Json(snapshot(&app)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowBody {
    #[serde(default)]
    image: BTreeMap<String, Threshold>,
    #[serde(default)]
    text: BTreeMap<String, Threshold>,
}

async fn put_workflow(
    Extract(app): Extract<Shared>,
    Path(name): Path<String>,
    Json(body): Json<WorkflowBody>,
) -> Result<Json<Value>, ApiError> {
    apply_edit(
        &app,
        Edit::SetWorkflow {
            name,
            image: body.image,
            text: body.text,
        },
    )
    .await?;
    Ok(Json(snapshot(&app)?))
}

async fn delete_workflow(
    Extract(app): Extract<Shared>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    apply_edit(&app, Edit::DeleteWorkflow { name }).await?;
    Ok(Json(snapshot(&app)?))
}

#[derive(Deserialize)]
struct KeyBody {
    name: String,
}

async fn create_key(
    Extract(app): Extract<Shared>,
    Json(body): Json<KeyBody>,
) -> Result<Json<Value>, ApiError> {
    let _one_at_a_time = app.edits.lock().await;
    let mut state = State::load(&app.state_path).map_err(ApiError::internal)?;
    let key = state
        .add_key(&body.name)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    state.save(&app.state_path).map_err(ApiError::internal)?;
    app.reload_keys().map_err(ApiError::internal)?;
    Ok(Json(json!({ "key": key, "state": snapshot(&app)? })))
}

async fn revoke_key(
    Extract(app): Extract<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let _one_at_a_time = app.edits.lock().await;
    let mut state = State::load(&app.state_path).map_err(ApiError::internal)?;
    if !state.revoke(&id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("no API key with id `{id}`"),
        ));
    }
    state.save(&app.state_path).map_err(ApiError::internal)?;
    app.reload_keys().map_err(ApiError::internal)?;
    Ok(Json(snapshot(&app)?))
}

async fn finish_setup(Extract(app): Extract<Shared>) -> Result<Json<Value>, ApiError> {
    let _one_at_a_time = app.edits.lock().await;
    let mut state = State::load(&app.state_path).map_err(ApiError::internal)?;
    state.setup_done = true;
    state.save(&app.state_path).map_err(ApiError::internal)?;
    Ok(Json(snapshot(&app)?))
}

async fn downloads(Extract(app): Extract<Shared>) -> Json<Downloads> {
    Json(
        app.downloads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    )
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DownloadBody {
    /// The tools to fetch; every enabled tool when absent.
    #[serde(default)]
    tools: Option<Vec<String>>,
}

async fn start_downloads(
    Extract(app): Extract<Shared>,
    Json(body): Json<DownloadBody>,
) -> Result<Json<Value>, ApiError> {
    let engine = app.engine();
    let wanted: Option<BTreeSet<String>> = body.tools.map(|t| t.into_iter().collect());
    let list = download::needed(&engine.config, wanted.as_ref());
    {
        let mut d = app.downloads.lock().unwrap_or_else(|e| e.into_inner());
        if d.running {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "busy",
                "downloads are already running",
            ));
        }
        if !list.is_empty() {
            *d = Downloads {
                running: true,
                files: list.iter().map(Progress::queued).collect(),
            };
        }
    }
    if !list.is_empty() {
        let worker = app.clone();
        std::thread::spawn(move || run_downloads(worker, list));
    }
    Ok(Json(snapshot(&app)?))
}

fn update(app: &Shared, i: usize, f: impl FnOnce(&mut Progress)) {
    if let Some(p) = app
        .downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .files
        .get_mut(i)
    {
        f(p);
    }
}

/// One file at a time, then a reload so the models that arrived start
/// working.
fn run_downloads(app: Shared, list: Vec<Needed>) {
    for (i, n) in list.iter().enumerate() {
        update(&app, i, |p| p.state = "downloading");
        let result = download::fetch(&n.url, &n.dest, &n.sha256, n.size, |done| {
            update(&app, i, |p| p.done = done)
        });
        update(&app, i, |p| match result {
            Ok(()) => {
                p.state = "done";
                p.done = p.size;
            }
            Err(e) => {
                tracing::error!(file = %n.name, error = %format!("{e:#}"), "download failed");
                p.state = "failed";
                p.error = Some(format!("{e:#}"));
            }
        });
    }
    {
        let _one_at_a_time = app.edits.blocking_lock();
        match Config::load(&app.config_path) {
            Ok(config) => app.reload(config),
            Err(e) => tracing::error!(error = %format!("{e:#}"), "reload after download failed"),
        }
    }
    app.downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .running = false;
}

async fn try_text(
    Extract(app): Extract<Shared>,
    Json(body): Json<TextBody>,
) -> Result<Json<Report>, ApiError> {
    let workflow = body.workflow.unwrap_or_else(|| "default".into());
    Ok(Json(
        run_check(&app, move |e| e.check_text(&body.text, &workflow)).await?,
    ))
}

async fn try_image(
    Extract(app): Extract<Shared>,
    Query(q): Query<WorkflowQuery>,
    body: Bytes,
) -> Result<Json<Report>, ApiError> {
    if body.is_empty() {
        return Err(ApiError::bad_request("empty body: send the image bytes"));
    }
    let workflow = q.workflow.unwrap_or_else(|| "default".into());
    Ok(Json(
        run_check(&app, move |e| e.check_image(&body, &workflow)).await?,
    ))
}

pub fn router(app: Shared, body_limit: usize) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.css", get(css))
        .route("/app.js", get(js))
        .route("/api/session", get(session))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/state", get(state))
        .route("/api/setup", post(finish_setup))
        .route("/api/tools/{id}", post(set_tool))
        .route("/api/downloads", get(downloads).post(start_downloads))
        .route(
            "/api/workflows/{name}",
            put(put_workflow).delete(delete_workflow),
        )
        .route("/api/keys", post(create_key))
        .route("/api/keys/{id}", delete(revoke_key))
        .route("/api/check/text", post(try_text))
        .route("/api/check/image", post(try_image))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(app)
}
