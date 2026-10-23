//! REST + WebSocket API.
//!
//! ```text
//! GET    /api/health
//! GET    /api/monitors
//! GET    /api/monitors/{ref}                 ref = key | id | number | name | alias
//! PATCH  /api/monitors/{ref}                 {"brightness": 40, "contrast": "+5", "input": "HDMI1", "power": true}
//! POST   /api/monitors/{ref}/refresh
//! GET    /api/monitors/{ref}/capabilities?refresh=true
//! GET    /api/monitors/{ref}/vcp/{code}
//! PUT    /api/monitors/{ref}/vcp/{code}      {"value": 50} | {"value": "+10"}
//! POST   /api/command                        {"command": "MateView b 30 N2724A b+10"}
//! POST   /api/rescan
//! GET    /api/system/volume
//! PUT    /api/system/volume                  {"volume": 30, "muted": false}
//! GET    /api/ws                             live events + commands
//! ```

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use lumen_core::engine::Monitor;
use lumen_core::patch::{MonitorPatch, ValueSpec};
use lumen_core::{platform, vcp, Engine, Event};
use serde::Deserialize;
use serde_json::{json, Value as Json_};
use tokio::sync::broadcast::error::RecvError;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    engine: Engine,
    token: Option<Arc<str>>,
}

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::BAD_REQUEST, format!("{e:#}"))
    }
}

type ApiResult<T> = Result<T, ApiError>;

pub fn router(engine: Engine, token: Option<String>) -> Router {
    let state = AppState { engine, token: token.filter(|t| !t.is_empty()).map(Into::into) };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/monitors", get(list_monitors))
        .route("/api/monitors/{mon}", get(get_monitor).patch(patch_monitor))
        .route("/api/monitors/{mon}/refresh", post(refresh_monitor))
        .route("/api/monitors/{mon}/capabilities", get(capabilities))
        .route("/api/monitors/{mon}/vcp/{code}", get(get_vcp).put(put_vcp))
        .route("/api/command", post(command))
        .route("/api/rescan", post(rescan))
        .route("/api/system/volume", get(get_volume).put(put_volume))
        .route("/api/ws", get(ws_upgrade))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let Some(token) = &s.token else {
        return next.run(req).await;
    };
    let header_ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == &**token);
    // Browsers cannot set headers on WebSocket upgrades, so `?token=` is accepted too.
    let query_ok = req
        .uri()
        .query()
        .into_iter()
        .flat_map(|q| q.split('&'))
        .any(|kv| kv.strip_prefix("token=") == Some(&**token));
    if header_ok || query_ok {
        next.run(req).await
    } else {
        ApiError(StatusCode::UNAUTHORIZED, "missing or invalid token".into()).into_response()
    }
}

/// Runs blocking engine work (DDC/CI calls take tens of milliseconds) off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(Into::into)
}

fn monitor(s: &AppState, reference: &str) -> ApiResult<Arc<Monitor>> {
    s.engine
        .resolve(reference)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("monitor '{reference}' not found")))
}

fn parse_code(code: &str) -> ApiResult<u8> {
    vcp::parse_number(code)
        .and_then(|c| u8::try_from(c).ok())
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, format!("invalid VCP code '{code}'")))
}

async fn health() -> Json<Json_> {
    Json(json!({ "name": "lumenctl", "version": env!("CARGO_PKG_VERSION") }))
}

async fn list_monitors(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.engine.monitors())
}

async fn get_monitor(State(s): State<AppState>, Path(mon): Path<String>) -> ApiResult<impl IntoResponse> {
    Ok(Json(monitor(&s, &mon)?.snapshot()))
}

async fn patch_monitor(
    State(s): State<AppState>,
    Path(mon): Path<String>,
    Json(patch): Json<MonitorPatch>,
) -> ApiResult<impl IntoResponse> {
    let m = monitor(&s, &mon)?;
    let engine = s.engine.clone();
    let snap = blocking(move || {
        engine.apply_patch(&m, &patch)?;
        Ok(m.snapshot())
    })
    .await?;
    Ok(Json(snap))
}

async fn refresh_monitor(State(s): State<AppState>, Path(mon): Path<String>) -> ApiResult<StatusCode> {
    monitor(&s, &mon)?.refresh();
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
struct CapsQuery {
    #[serde(default)]
    refresh: bool,
}

async fn capabilities(
    State(s): State<AppState>,
    Path(mon): Path<String>,
    Query(q): Query<CapsQuery>,
) -> ApiResult<impl IntoResponse> {
    let m = monitor(&s, &mon)?;
    Ok(Json(blocking(move || m.capabilities(q.refresh)).await?))
}

async fn get_vcp(
    State(s): State<AppState>,
    Path((mon, code)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let m = monitor(&s, &mon)?;
    let code = parse_code(&code)?;
    let v = blocking(move || m.read(code)).await?;
    Ok(Json(json!({ "code": code, "name": vcp::vcp_name(code), "current": v.current, "max": v.max })))
}

#[derive(Deserialize)]
struct VcpBody {
    value: ValueSpec,
}

async fn put_vcp(
    State(s): State<AppState>,
    Path((mon, code)): Path<(String, String)>,
    Json(body): Json<VcpBody>,
) -> ApiResult<impl IntoResponse> {
    let m = monitor(&s, &mon)?;
    let code = parse_code(&code)?;
    let engine = s.engine.clone();
    let value = blocking(move || engine.set_vcp(&m, code, body.value.0)).await?;
    Ok(Json(json!({ "code": code, "value": value })))
}

#[derive(Deserialize)]
struct CommandBody {
    command: String,
}

async fn command(State(s): State<AppState>, Json(body): Json<CommandBody>) -> ApiResult<impl IntoResponse> {
    let engine = s.engine.clone();
    Ok(Json(blocking(move || engine.execute(&body.command)).await?))
}

async fn rescan(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let engine = s.engine.clone();
    blocking(move || engine.rescan()).await?;
    Ok(Json(s.engine.monitors()))
}

async fn get_volume() -> ApiResult<impl IntoResponse> {
    let (volume, muted) = blocking(platform::system_volume).await?;
    Ok(Json(json!({ "volume": volume, "muted": muted })))
}

#[derive(Deserialize)]
struct VolumeBody {
    volume: Option<ValueSpec>,
    muted: Option<bool>,
}

async fn put_volume(State(s): State<AppState>, Json(body): Json<VolumeBody>) -> ApiResult<impl IntoResponse> {
    let engine = s.engine.clone();
    blocking(move || {
        if let Some(v) = body.volume {
            engine.set_system_volume(v.0)?;
        }
        if let Some(m) = body.muted {
            engine.set_system_mute(m)?;
        }
        Ok(())
    })
    .await?;
    get_volume().await
}

async fn ws_upgrade(State(s): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| ws_session(socket, s.engine))
}

/// Client messages:
/// `{"type":"command","command":"b 50","id":1}`
/// `{"type":"set","monitor":"1","brightness":40,"id":2}`
/// `{"type":"rescan"}`
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Command { command: String, id: Option<Json_> },
    Set {
        monitor: String,
        id: Option<Json_>,
        #[serde(flatten)]
        patch: MonitorPatch,
    },
    Rescan { id: Option<Json_> },
}

async fn ws_session(socket: WebSocket, engine: Engine) {
    let (mut sink, mut stream) = socket.split();
    let mut events = engine.subscribe();
    let hello = Event::MonitorsChanged { monitors: engine.monitors() };
    if sink.send(Message::Text(serde_json::to_string(&hello).unwrap().into())).await.is_err() {
        return;
    }
    let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel::<Json_>(32);

    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(ev) => {
                    let text = serde_json::to_string(&ev).unwrap();
                    if sink.send(Message::Text(text.into())).await.is_err() { break; }
                }
                Err(RecvError::Lagged(_)) => {
                    let ev = Event::MonitorsChanged { monitors: engine.monitors() };
                    let text = serde_json::to_string(&ev).unwrap();
                    if sink.send(Message::Text(text.into())).await.is_err() { break; }
                }
                Err(RecvError::Closed) => break,
            },
            Some(reply) = reply_rx.recv() => {
                if sink.send(Message::Text(reply.to_string().into())).await.is_err() { break; }
            }
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    let engine = engine.clone();
                    let tx = reply_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = tx.blocking_send(handle_client(&engine, &text));
                    });
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

fn handle_client(engine: &Engine, text: &str) -> Json_ {
    let msg: ClientMsg = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => return json!({ "type": "error", "error": e.to_string() }),
    };
    let (id, result) = match msg {
        ClientMsg::Command { command, id } => {
            (id, engine.execute(&command).map(|o| serde_json::to_value(o).unwrap()))
        }
        ClientMsg::Set { monitor, id, patch } => (
            id,
            engine
                .resolve(&monitor)
                .ok_or_else(|| anyhow::anyhow!("monitor '{monitor}' not found"))
                .and_then(|m| {
                    engine.apply_patch(&m, &patch)?;
                    Ok(serde_json::to_value(m.snapshot()).unwrap())
                }),
        ),
        ClientMsg::Rescan { id } => (id, engine.rescan().map(|_| json!(engine.monitors()))),
    };
    match result {
        Ok(v) => json!({ "type": "result", "id": id, "ok": true, "result": v }),
        Err(e) => json!({ "type": "result", "id": id, "ok": false, "error": format!("{e:#}") }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use lumen_core::backend::mock::MockBackend;
    use lumen_core::Config;
    use tower::ServiceExt;

    fn engine() -> Engine {
        let e = Engine::new(Config::default(), None, Box::new(MockBackend::with_monitors(2)));
        e.rescan().unwrap();
        for m in e.monitor_handles() {
            m.read(vcp::BRIGHTNESS).unwrap();
        }
        e
    }

    async fn call(app: &Router, method: &str, uri: &str, body: Option<Json_>, token: Option<&str>) -> (StatusCode, Json_) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let req = match body {
            Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Json_::Null))
    }

    #[tokio::test]
    async fn list_patch_and_command() {
        let app = router(engine(), None);
        let (st, list) = call(&app, "GET", "/api/monitors", None, None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(list.as_array().unwrap().len(), 2);
        let key = list[1]["key"].as_str().unwrap().to_string();

        let (st, m) = call(&app, "PATCH", &format!("/api/monitors/{key}"), Some(json!({"brightness": "+20"})), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(m["brightness"]["current"], 70);

        let (st, out) = call(&app, "POST", "/api/command", Some(json!({"command": "Mock1 b 5 d"})), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(out["value"], 5);

        let (st, v) = call(&app, "GET", "/api/monitors/1/vcp/0x10", None, None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["current"], 5);

        let (st, _) = call(&app, "GET", "/api/monitors/nope", None, None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn token_is_enforced() {
        let app = router(engine(), Some("s3cret".into()));
        assert_eq!(call(&app, "GET", "/api/monitors", None, None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, "GET", "/api/monitors", None, Some("bad")).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, "GET", "/api/monitors", None, Some("s3cret")).await.0, StatusCode::OK);
        assert_eq!(call(&app, "GET", "/api/monitors?token=s3cret", None, None).await.0, StatusCode::OK);
    }
}
