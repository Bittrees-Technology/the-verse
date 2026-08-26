// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderValue, Method, StatusCode, header::CONTENT_TYPE},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::broadcast;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info, warn};
use verse_protocol::{ClientMessage, PROTOCOL_VERSION, ServerMessage, WorldSnapshot};
use verse_simulation::{IntentError, Runtime, RuntimeError};

const COMMAND_CENTER_HTML: &str = include_str!("../../../apps/web-command-center/index.html");
const COMMAND_CENTER_JS: &str = include_str!("../../../apps/web-command-center/app.js");
const COMMAND_CENTER_CSS: &str = include_str!("../../../apps/web-command-center/styles.css");

#[derive(Debug)]
pub struct AppState {
    runtime: Mutex<Runtime>,
    updates: broadcast::Sender<ServerMessage>,
}

impl AppState {
    pub fn new(runtime: Runtime) -> Arc<Self> {
        let (updates, _) = broadcast::channel(64);
        Arc::new(Self {
            runtime: Mutex::new(runtime),
            updates,
        })
    }

    pub fn snapshot(&self) -> WorldSnapshot {
        self.runtime.lock().snapshot()
    }

    pub fn persist_snapshot(&self) -> Result<(), RuntimeError> {
        self.runtime.lock().persist_snapshot()
    }

    pub fn advance(&self, delta_millis: u16) -> Result<bool, RuntimeError> {
        let mut runtime = self.runtime.lock();
        let changed = runtime.advance(delta_millis)?;
        if changed {
            let _ = self.updates.send(ServerMessage::Snapshot {
                snapshot: Box::new(runtime.snapshot()),
            });
        }
        Ok(changed)
    }
}

#[derive(Debug, Serialize)]
struct StatusDocument {
    service: &'static str,
    protocol_version: u32,
    content_manifest_version: String,
    universe_id: String,
    cell_id: String,
    event_sequence: u64,
    simulation_tick: u64,
    fencing_token: u64,
    world_hash: String,
    conservation_valid: bool,
}

pub fn router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(HeaderValue::from_static("http://localhost:3000"))
        .allow_methods([Method::GET])
        .allow_headers([]);
    Router::new()
        .route("/", get(command_center))
        .route("/app.js", get(command_center_javascript))
        .route("/styles.css", get(command_center_styles))
        .route("/healthz", get(health))
        .route("/api/v1/status", get(status))
        .route("/api/v1/world", get(world))
        .route("/ws", get(websocket_upgrade))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(false)),
        )
        .layer(cors)
        .with_state(state)
}

async fn command_center() -> Html<&'static str> {
    Html(COMMAND_CENTER_HTML)
}

async fn command_center_javascript() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
        COMMAND_CENTER_JS,
    )
}

async fn command_center_styles() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/css; charset=utf-8")],
        COMMAND_CENTER_CSS,
    )
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusDocument> {
    let snapshot = state.snapshot();
    Json(StatusDocument {
        service: "verse-simulation-worker",
        protocol_version: PROTOCOL_VERSION,
        content_manifest_version: snapshot.content_manifest_version,
        universe_id: snapshot.universe_id,
        cell_id: snapshot.cell_id,
        event_sequence: snapshot.event_sequence,
        simulation_tick: snapshot.simulation_tick,
        fencing_token: snapshot.fencing_token,
        world_hash: snapshot.world_hash,
        conservation_valid: snapshot.conservation.valid,
    })
}

async fn world(State(state): State<Arc<AppState>>) -> Json<WorldSnapshot> {
    Json(state.snapshot())
}

async fn websocket_upgrade(
    upgrade: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    upgrade
        .max_message_size(8 * 1024 * 1024)
        .on_upgrade(move |socket| websocket_session(socket, state))
}

async fn websocket_session(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    if send_server_message(
        &mut sender,
        &ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            server_name: "The Verse local universe".into(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    if send_server_message(
        &mut sender,
        &ServerMessage::Snapshot {
            snapshot: Box::new(state.snapshot()),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    let mut updates = state.updates.subscribe();
    loop {
        tokio::select! {
            client_message = receiver.next() => {
                match client_message {
                    Some(Ok(message)) => {
                        if !handle_client_message(message, &state, &mut sender).await {
                            break;
                        }
                    }
                    Some(Err(source)) => {
                        warn!(%source, "websocket receive failed");
                        break;
                    }
                    None => break,
                }
            }
            update = updates.recv() => {
                match update {
                    Ok(message) => {
                        if send_server_message(&mut sender, &message).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "client lagged; sending a complete snapshot");
                        if send_server_message(
                            &mut sender,
                            &ServerMessage::Snapshot {
                                snapshot: Box::new(state.snapshot()),
                            },
                        ).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn handle_client_message(
    message: Message,
    state: &Arc<AppState>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> bool {
    let Message::Text(text) = message else {
        return !matches!(message, Message::Close(_));
    };
    let parsed = match serde_json::from_str::<ClientMessage>(&text) {
        Ok(message) => message,
        Err(source) => {
            return send_server_message(
                sender,
                &ServerMessage::IntentRejected {
                    operation_id: None,
                    code: "invalid_json".into(),
                    message: source.to_string(),
                },
            )
            .await
            .is_ok();
        }
    };

    match parsed {
        ClientMessage::Hello {
            protocol_version,
            client_name,
        } => {
            if protocol_version != PROTOCOL_VERSION {
                return send_server_message(
                    sender,
                    &ServerMessage::Fatal {
                        code: "protocol_version_mismatch".into(),
                        message: format!(
                            "server requires protocol {PROTOCOL_VERSION}, client sent {protocol_version}"
                        ),
                    },
                )
                .await
                .is_ok();
            }
            info!(%client_name, "client completed protocol handshake");
            true
        }
        ClientMessage::RequestSnapshot => send_server_message(
            sender,
            &ServerMessage::Snapshot {
                snapshot: Box::new(state.snapshot()),
            },
        )
        .await
        .is_ok(),
        intent => {
            let result = state.runtime.lock().execute(&intent);
            match result {
                Ok(receipt) => {
                    if send_server_message(sender, &ServerMessage::IntentAccepted { receipt })
                        .await
                        .is_err()
                    {
                        return false;
                    }
                    let _ = state.updates.send(ServerMessage::Snapshot {
                        snapshot: Box::new(state.snapshot()),
                    });
                    true
                }
                Err(RuntimeError::Intent(source)) => {
                    send_intent_error(sender, intent.operation_id(), source).await
                }
                Err(RuntimeError::Persistence(source)) => {
                    error!(%source, "persistence failure while processing intent");
                    send_server_message(
                        sender,
                        &ServerMessage::Fatal {
                            code: "persistence_failure".into(),
                            message: "authoritative persistence failed; writes are stopped".into(),
                        },
                    )
                    .await
                    .is_ok()
                }
                Err(RuntimeError::Halted) => send_server_message(
                    sender,
                    &ServerMessage::Fatal {
                        code: "authoritative_writes_halted".into(),
                        message: "the universe is in fail-closed recovery mode".into(),
                    },
                )
                .await
                .is_ok(),
            }
        }
    }
}

async fn send_intent_error(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    operation_id: Option<&str>,
    source: IntentError,
) -> bool {
    send_server_message(
        sender,
        &ServerMessage::IntentRejected {
            operation_id: operation_id.map(ToOwned::to_owned),
            code: source.code().into(),
            message: source.to_string(),
        },
    )
    .await
    .is_ok()
}

async fn send_server_message(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &ServerMessage,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).map_err(axum::Error::new)?;
    sender.send(Message::Text(text.into())).await
}

pub fn internal_error(source: impl std::fmt::Display) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("authoritative service failure: {source}"),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use super::*;

    fn test_app() -> Router {
        let directory = tempdir().expect("tempdir").keep();
        let runtime = Runtime::open(directory, 99, 20).expect("runtime");
        router(AppState::new(runtime))
    }

    #[tokio::test]
    async fn status_reports_conserved_genesis() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(json["conservation_valid"], true);
        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn health_endpoint_is_ready() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn command_center_assets_are_served_by_the_game_server() {
        for (uri, expected_content_type) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/styles.css", "text/css"),
        ] {
            let response = test_app()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.starts_with(expected_content_type))
            );
        }
    }
}
