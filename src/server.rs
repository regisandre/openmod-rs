//! HTTP API, including SightEngine-compatible workflow endpoints, and the
//! shared state the admin interface edits.

use crate::admin::{self, Sessions};
use crate::config::Config;
use crate::download::Downloads;
use crate::engine::{CheckError, Engine, Report};
use crate::keys::{KeySet, LiveKeys, State};
use anyhow::{Context, Result, bail};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State as Extract};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::Semaphore;

const QUEUE_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything both listeners share. The engine and the keys are swapped
/// whole when the admin interface changes them, so a request always sees
/// one consistent version.
pub struct App {
    engine: RwLock<Arc<Engine>>,
    keys: Mutex<LiveKeys>,
    permits: Semaphore,
    pub config_path: PathBuf,
    pub state_path: PathBuf,
    allow_unauthenticated: bool,
    /// Serializes configuration edits and reloads.
    pub edits: tokio::sync::Mutex<()>,
    pub sessions: Sessions,
    pub downloads: Mutex<Downloads>,
}

pub type Shared = Arc<App>;

impl App {
    pub fn engine(&self) -> Arc<Engine> {
        self.engine
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Blocking: builds a new engine from `config`, keeping the models whose
    /// settings did not change, then swaps it in. Checks in flight finish on
    /// the old one.
    pub fn reload(&self, config: Config) {
        let previous = self.engine();
        let next = Engine::load(config, Some(&previous));
        *self.engine.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(next);
    }

    /// Re-reads the external keys and the state file.
    pub fn reload_keys(&self) -> Result<()> {
        let engine = self.engine();
        let external = KeySet::external(engine.config.server.api_keys_file.as_deref())?;
        self.keys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reload(external)
    }

    pub fn key_count(&self) -> usize {
        self.keys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current()
            .len()
    }

    fn require_key(&self, presented: Option<&str>) -> Result<(), ApiError> {
        let mut live = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        let keys = live.current();
        if keys.is_empty() {
            if self.allow_unauthenticated {
                return Ok(());
            }
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "no API key exists on this server yet: create one in the admin interface",
            ));
        }
        match presented {
            Some(p) if keys.matches(p) => Ok(()),
            _ => Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing or invalid API key",
            )),
        }
    }
}

/// An API error with a stable `type` a client can switch on.
pub struct ApiError {
    status: StatusCode,
    kind: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            kind,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error, see the server log",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "status": "failure",
            "error": { "type": self.kind, "code": self.status.as_u16(), "message": self.message }
        });
        (self.status, Json(body)).into_response()
    }
}

impl From<CheckError> for ApiError {
    fn from(e: CheckError) -> Self {
        use crate::image_input::DecodeError;
        let message = e.to_string();
        match e {
            CheckError::UnknownWorkflow(_) => {
                ApiError::new(StatusCode::BAD_REQUEST, "unknown_workflow", message)
            }
            CheckError::Unavailable(_) => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "workflow_unavailable",
                message,
            ),
            CheckError::Decode(DecodeError::Unsupported) => ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media",
                message,
            ),
            CheckError::Decode(DecodeError::TooLarge(_)) => {
                ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "image_too_large", message)
            }
            CheckError::Decode(DecodeError::Corrupt(_)) => {
                ApiError::new(StatusCode::BAD_REQUEST, "corrupt_media", message)
            }
            CheckError::TextTooLong(_) => {
                ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "text_too_long", message)
            }
            CheckError::Inference(err) => {
                tracing::error!(error = %err, "inference failed");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "inference failed",
                )
            }
        }
    }
}

fn header_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
}

/// Runs a blocking check on the worker pool, bounded by `max_concurrent`.
pub async fn run_check<F>(app: &App, f: F) -> Result<Report, ApiError>
where
    F: FnOnce(&Engine) -> Result<Report, CheckError> + Send + 'static,
{
    let permit = tokio::time::timeout(QUEUE_TIMEOUT, app.permits.acquire())
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "busy",
                "the server is busy, retry later",
            )
        })?
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "busy",
                "the server is shutting down",
            )
        })?;
    let engine = app.engine();
    let result = tokio::task::spawn_blocking(move || f(&engine))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "check crashed",
            )
        })?;
    drop(permit);
    Ok(result?)
}

#[derive(Deserialize)]
pub struct WorkflowQuery {
    pub workflow: Option<String>,
}

#[derive(Default)]
struct Form {
    media: Option<Bytes>,
    text: Option<String>,
    workflow: Option<String>,
    api_secret: Option<String>,
}

async fn read_form(mut multipart: Multipart) -> Result<Form, ApiError> {
    let bad = |m: String| ApiError::bad_request(m);
    let mut form = Form::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "media" => form.media = Some(field.bytes().await.map_err(|e| bad(e.to_string()))?),
            "text" => form.text = Some(field.text().await.map_err(|e| bad(e.to_string()))?),
            "workflow" => form.workflow = Some(field.text().await.map_err(|e| bad(e.to_string()))?),
            "api_secret" => {
                form.api_secret = Some(field.text().await.map_err(|e| bad(e.to_string()))?)
            }
            _ => {}
        }
    }
    Ok(form)
}

/// `{"status": "ok"}`, or `"degraded"` with the enabled models that are not
/// ready.
async fn health(Extract(app): Extract<Shared>) -> impl IntoResponse {
    let engine = app.engine();
    let problems: Vec<String> = engine
        .status
        .iter()
        .filter(|(_, s)| {
            !matches!(
                s,
                crate::engine::ToolStatus::Ready | crate::engine::ToolStatus::Disabled
            )
        })
        .map(|(name, _)| format!("{name} is not ready"))
        .collect();
    let status = if problems.is_empty() {
        "ok"
    } else {
        "degraded"
    };
    Json(json!({ "status": status, "version": env!("CARGO_PKG_VERSION"), "problems": problems }))
}

/// `POST /v1/image?workflow=default` with the raw image as the body.
async fn check_image(
    Extract(app): Extract<Shared>,
    headers: HeaderMap,
    Query(q): Query<WorkflowQuery>,
    body: Bytes,
) -> Result<Json<Report>, ApiError> {
    app.require_key(header_key(&headers).as_deref())?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty body: send the image bytes"));
    }
    let workflow = q.workflow.unwrap_or_else(|| "default".into());
    Ok(Json(
        run_check(&app, move |e| e.check_image(&body, &workflow)).await?,
    ))
}

#[derive(Deserialize)]
pub struct TextBody {
    pub text: String,
    pub workflow: Option<String>,
}

/// `POST /v1/text` with `{"text": "...", "workflow": "default"}`.
async fn check_text(
    Extract(app): Extract<Shared>,
    headers: HeaderMap,
    Json(body): Json<TextBody>,
) -> Result<Json<Report>, ApiError> {
    app.require_key(header_key(&headers).as_deref())?;
    let workflow = body.workflow.unwrap_or_else(|| "default".into());
    Ok(Json(
        run_check(&app, move |e| e.check_text(&body.text, &workflow)).await?,
    ))
}

/// SightEngine-compatible `POST /1.0/check-workflow.json` (multipart with
/// `media`, `workflow`, `api_user`, `api_secret`).
async fn compat_image(
    Extract(app): Extract<Shared>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<Report>, ApiError> {
    let form = read_form(multipart).await?;
    let key = form.api_secret.clone().or_else(|| header_key(&headers));
    app.require_key(key.as_deref())?;
    let media = form
        .media
        .ok_or_else(|| ApiError::bad_request("missing `media` field"))?;
    let workflow = form.workflow.unwrap_or_else(|| "default".into());
    Ok(Json(
        run_check(&app, move |e| e.check_image(&media, &workflow)).await?,
    ))
}

/// SightEngine-compatible `POST /1.0/text/check-workflow.json`.
async fn compat_text(
    Extract(app): Extract<Shared>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<Report>, ApiError> {
    let form = read_form(multipart).await?;
    let key = form.api_secret.clone().or_else(|| header_key(&headers));
    app.require_key(key.as_deref())?;
    let text = form
        .text
        .ok_or_else(|| ApiError::bad_request("missing `text` field"))?;
    let workflow = form.workflow.unwrap_or_else(|| "default".into());
    Ok(Json(
        run_check(&app, move |e| e.check_text(&text, &workflow)).await?,
    ))
}

/// Downloads again the lists older than `hours` that changed upstream, then
/// reloads the engine so the new lists count. Skipped while the admin
/// interface downloads files.
fn refresh_lists(app: &App, hours: u64) {
    if app
        .downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .running
    {
        return;
    }
    let config = app.engine().config.clone();
    if crate::download::refresh_lists(&config, Duration::from_secs(hours * 3600)) == 0 {
        return;
    }
    let _one_at_a_time = app.edits.blocking_lock();
    match Config::load(&app.config_path) {
        Ok(config) => app.reload(config),
        Err(e) => tracing::error!(error = %format!("{e:#}"), "reload after a list refresh failed"),
    }
}

/// Resolves on Ctrl+C, and on SIGTERM where there is one (`docker stop`).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("installing the SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

fn banner(token: &str, admin: &SocketAddr) {
    let host = if admin.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        admin.ip().to_string()
    };
    eprintln!();
    eprintln!("  openmod admin interface: http://{host}:{}", admin.port());
    eprintln!("  admin token (shown once): {token}");
    eprintln!("  lost it? run `openmod admin-token` for a new one.");
    eprintln!();
}

pub async fn serve(config_path: PathBuf, engine: Engine) -> Result<()> {
    let config = engine.config.clone();
    let api_addr: SocketAddr = config
        .server
        .bind
        .parse()
        .with_context(|| format!("server.bind `{}`", config.server.bind))?;
    let state_path = config.server.state_file.clone();
    let mut state = State::load(&state_path)?;

    let admin_addr: Option<SocketAddr> = if config.admin.enabled {
        Some(
            config
                .admin
                .bind
                .parse()
                .with_context(|| format!("admin.bind `{}`", config.admin.bind))?,
        )
    } else {
        None
    };
    if let Some(addr) = &admin_addr {
        if state.admin_digest().is_none() {
            let token = state.rotate_admin_token();
            state.save(&state_path)?;
            banner(&token, addr);
        }
        if !addr.ip().is_loopback() {
            tracing::warn!(
                "the admin interface listens on {addr}: keep it off the internet (SSH tunnel, or an HTTPS reverse proxy with access control)"
            );
        }
    }

    let mut keys = LiveKeys::load(
        KeySet::external(config.server.api_keys_file.as_deref())?,
        &state_path,
    )?;
    if keys.current().is_empty() {
        if config.server.allow_unauthenticated {
            if !api_addr.ip().is_loopback() {
                bail!("refusing to serve without API keys on a non-loopback address ({api_addr})");
            }
            tracing::warn!("serving WITHOUT authentication on {api_addr}");
        } else {
            tracing::warn!("no API key yet: every API request is refused until you create one");
        }
    }

    let limit = config.server.max_upload_bytes;
    let app: Shared = Arc::new(App {
        engine: RwLock::new(Arc::new(engine)),
        keys: Mutex::new(keys),
        permits: Semaphore::new(config.server.max_concurrent.max(1)),
        config_path,
        state_path,
        allow_unauthenticated: config.server.allow_unauthenticated,
        edits: tokio::sync::Mutex::new(()),
        sessions: Sessions::new(admin::SESSION_TTL),
        downloads: Mutex::new(Downloads::default()),
    });

    let hours = config.server.lists_refresh_hours;
    if hours > 0 {
        let worker = app.clone();
        tokio::spawn(async move {
            // Hourly: each list is asked for again once it is older than
            // `lists_refresh_hours`, so a restart never delays a refresh.
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let app = worker.clone();
                let _ = tokio::task::spawn_blocking(move || refresh_lists(&app, hours)).await;
            }
        });
    }

    let api = Router::new()
        .route("/health", get(health))
        .route("/v1/image", post(check_image))
        .route("/v1/text", post(check_text))
        .route("/1.0/check-workflow.json", post(compat_image))
        .route("/1.0/text/check-workflow.json", post(compat_text))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(app.clone());
    let listener = tokio::net::TcpListener::bind(api_addr)
        .await
        .with_context(|| format!("binding {api_addr}"))?;
    tracing::info!("API listening on http://{api_addr}");
    let api = async move {
        axum::serve(listener, api)
            .with_graceful_shutdown(shutdown_signal())
            .await
    };

    match admin_addr {
        Some(addr) => {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("binding {addr}"))?;
            tracing::info!("admin interface on http://{addr}");
            let router = admin::router(app.clone(), limit);
            let admin = async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(shutdown_signal())
                    .await
            };
            tokio::try_join!(api, admin)?;
        }
        None => api.await?,
    }
    Ok(())
}
