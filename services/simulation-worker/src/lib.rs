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
use verse_protocol::{
    ClientMessage, MotionSnapshot, PROTOCOL_VERSION, ServerMessage, WorldSnapshot,
};
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

    pub fn motion_snapshot(&self) -> MotionSnapshot {
        self.runtime.lock().motion_snapshot()
    }

    pub fn persist_snapshot(&self) -> Result<(), RuntimeError> {
        self.runtime.lock().persist_snapshot()
    }

    pub fn is_halted(&self) -> bool {
        self.runtime.lock().is_halted()
    }

    pub fn advance(&self, delta_millis: u16) -> Result<bool, RuntimeError> {
        let mut runtime = self.runtime.lock();
        let before_life_state = runtime.state().player.life_state.clone();
        let before_oxygen = runtime.state().player.suit_oxygen_milli;
        let changed = runtime.advance(delta_millis)?;
        if changed {
            let lifecycle_changed = runtime.state().player.life_state != before_life_state
                || runtime.state().player.suit_oxygen_milli != before_oxygen;
            let update = if lifecycle_changed {
                ServerMessage::Snapshot {
                    snapshot: Box::new(runtime.snapshot()),
                }
            } else {
                ServerMessage::MotionState {
                    motion: Box::new(runtime.motion_snapshot()),
                }
            };
            let _ = self.updates.send(update);
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
    authoritative_halted: bool,
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

async fn health(State(state): State<Arc<AppState>>) -> StatusCode {
    if state.is_halted() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::NO_CONTENT
    }
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
        authoritative_halted: state.is_halted(),
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
    let Some(client_name) = complete_handshake(&mut receiver, &mut sender).await else {
        return;
    };
    info!(%client_name, "client completed protocol handshake");

    let mut updates = state.updates.subscribe();
    let initial_snapshot = state.snapshot();
    let mut last_sent_event_sequence = initial_snapshot.event_sequence;
    if send_server_message(
        &mut sender,
        &ServerMessage::Snapshot {
            snapshot: Box::new(initial_snapshot),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            client_message = receiver.next() => {
                match client_message {
                    Some(Ok(message)) => {
                        if !handle_client_message(
                            message,
                            &state,
                            &mut sender,
                            &mut last_sent_event_sequence,
                        ).await {
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
                        let event_sequence = match &message {
                            ServerMessage::Snapshot { snapshot } => Some(snapshot.event_sequence),
                            ServerMessage::MotionState { motion } => Some(motion.event_sequence),
                            _ => None,
                        };
                        if let Some(event_sequence) = event_sequence {
                            if event_sequence <= last_sent_event_sequence {
                                continue;
                            }
                            last_sent_event_sequence = event_sequence;
                        }
                        if send_server_message(&mut sender, &message).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "client lagged; sending a complete snapshot");
                        let snapshot = state.snapshot();
                        last_sent_event_sequence = snapshot.event_sequence;
                        if send_server_message(
                            &mut sender,
                            &ServerMessage::Snapshot {
                                snapshot: Box::new(snapshot),
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

async fn complete_handshake(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> Option<String> {
    loop {
        let message = match receiver.next().await {
            Some(Ok(message)) => message,
            Some(Err(source)) => {
                warn!(%source, "websocket handshake receive failed");
                return None;
            }
            None => return None,
        };
        match message {
            Message::Text(text) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(ClientMessage::Hello {
                    protocol_version,
                    client_name,
                }) if protocol_version == PROTOCOL_VERSION => return Some(client_name),
                Ok(ClientMessage::Hello {
                    protocol_version, ..
                }) => {
                    send_fatal_and_close(
                        sender,
                        "protocol_version_mismatch",
                        format!(
                            "server requires protocol {PROTOCOL_VERSION}, client sent {protocol_version}"
                        ),
                    )
                    .await;
                    return None;
                }
                Ok(_) => {
                    send_fatal_and_close(
                        sender,
                        "protocol_handshake_required",
                        "send a compatible hello before requesting state or submitting intents",
                    )
                    .await;
                    return None;
                }
                Err(source) => {
                    send_fatal_and_close(sender, "invalid_handshake", source.to_string()).await;
                    return None;
                }
            },
            Message::Ping(payload) => {
                if sender.send(Message::Pong(payload)).await.is_err() {
                    return None;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => return None,
            Message::Binary(_) => {
                send_fatal_and_close(
                    sender,
                    "invalid_handshake",
                    "the protocol handshake must be a JSON text message",
                )
                .await;
                return None;
            }
        }
    }
}

async fn handle_client_message(
    message: Message,
    state: &Arc<AppState>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    last_sent_event_sequence: &mut u64,
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
        ClientMessage::Hello { .. } => {
            send_fatal_and_close(
                sender,
                "protocol_handshake_already_complete",
                "a connection may complete the protocol handshake only once",
            )
            .await;
            false
        }
        ClientMessage::RequestSnapshot => {
            let snapshot = state.snapshot();
            *last_sent_event_sequence = snapshot.event_sequence;
            send_server_message(
                sender,
                &ServerMessage::Snapshot {
                    snapshot: Box::new(snapshot),
                },
            )
            .await
            .is_ok()
        }
        intent => {
            let player_control = matches!(&intent, ClientMessage::SetPlayerControl { .. });
            let result = state.runtime.lock().execute(&intent);
            match result {
                Ok(receipt) => {
                    if send_server_message(sender, &ServerMessage::IntentAccepted { receipt })
                        .await
                        .is_err()
                    {
                        return false;
                    }
                    let update = if player_control {
                        ServerMessage::MotionState {
                            motion: Box::new(state.motion_snapshot()),
                        }
                    } else {
                        ServerMessage::Snapshot {
                            snapshot: Box::new(state.snapshot()),
                        }
                    };
                    let _ = state.updates.send(update);
                    true
                }
                Err(RuntimeError::Intent(source)) => {
                    send_intent_error(sender, intent.operation_id(), source).await
                }
                Err(RuntimeError::Persistence(source)) => {
                    error!(%source, "persistence failure while processing intent");
                    send_fatal_and_close(
                        sender,
                        "persistence_failure",
                        "authoritative persistence failed; writes are stopped",
                    )
                    .await;
                    false
                }
                Err(RuntimeError::Physics(source)) => {
                    error!(%source, "physics failure while processing intent");
                    send_fatal_and_close(
                        sender,
                        "physics_failure",
                        "authoritative physics rejected the operation",
                    )
                    .await;
                    false
                }
                Err(RuntimeError::Halted) => {
                    send_fatal_and_close(
                        sender,
                        "authoritative_writes_halted",
                        "the universe is in fail-closed recovery mode",
                    )
                    .await;
                    false
                }
            }
        }
    }
}

async fn send_intent_error(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    operation_id: Option<&str>,
    source: IntentError,
) -> bool {
    let message = source.message();
    send_server_message(
        sender,
        &ServerMessage::IntentRejected {
            operation_id: operation_id.map(ToOwned::to_owned),
            code: source.code().into(),
            message,
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

async fn send_fatal_and_close(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    code: &str,
    message: impl Into<String>,
) {
    let _ = send_server_message(
        sender,
        &ServerMessage::Fatal {
            code: code.into(),
            message: message.into(),
        },
    )
    .await;
    let _ = sender.send(Message::Close(None)).await;
}

pub fn internal_error(source: impl std::fmt::Display) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("authoritative service failure: {source}"),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use futures_util::{SinkExt as _, StreamExt as _};
    use http::Request;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message as ClientWebSocketMessage;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
    use tower::ServiceExt;

    use super::*;

    fn test_app() -> Router {
        router(test_state())
    }

    fn test_state() -> Arc<AppState> {
        let directory = tempdir().expect("tempdir").keep();
        AppState::new(Runtime::open(directory, 99, 20).expect("runtime"))
    }

    type TestSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

    async fn connect_test_socket() -> (TestSocket, Arc<AppState>, tokio::task::JoinHandle<()>) {
        let state = test_state();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        let address = listener.local_addr().expect("listener has address");
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router(server_state))
                .await
                .expect("test server runs");
        });
        let (socket, _) = connect_async(format!("ws://{address}/ws"))
            .await
            .expect("test websocket connects");
        (socket, state, server)
    }

    async fn receive_server_message(socket: &mut TestSocket) -> ServerMessage {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .expect("server message arrives before timeout")
                .expect("websocket remains open")
                .expect("websocket message is valid");
            if let ClientWebSocketMessage::Text(text) = message {
                return serde_json::from_str(&text).expect("server message is valid protocol JSON");
            }
        }
    }

    async fn send_client_message(socket: &mut TestSocket, message: &ClientMessage) {
        socket
            .send(ClientWebSocketMessage::Text(
                serde_json::to_string(message)
                    .expect("client message serializes")
                    .into(),
            ))
            .await
            .expect("client message sends");
    }

    async fn assert_socket_closes(socket: &mut TestSocket) {
        let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("server closes incompatible connection promptly");
        assert!(matches!(
            next,
            None | Some(Ok(ClientWebSocketMessage::Close(_)))
        ));
    }

    #[tokio::test]
    async fn websocket_requires_hello_before_snapshot_or_mutation() {
        let (mut socket, state, server) = connect_test_socket().await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome { .. }
        ));
        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
                operation_id: "pre-handshake-mutation".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::ZERO,
                angular_input: verse_protocol::Vec3::ZERO,
                boost: false,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Fatal { ref code, .. } if code == "protocol_handshake_required"
        ));
        assert_eq!(state.snapshot().event_sequence, 0);
        assert_socket_closes(&mut socket).await;
        server.abort();
    }

    #[tokio::test]
    async fn websocket_rejects_and_closes_an_incompatible_protocol() {
        let (mut socket, state, server) = connect_test_socket().await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome { .. }
        ));
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION - 1,
                client_name: "obsolete-client".into(),
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Fatal { ref code, .. } if code == "protocol_version_mismatch"
        ));
        assert_eq!(state.snapshot().event_sequence, 0);
        assert_socket_closes(&mut socket).await;
        server.abort();
    }

    #[tokio::test]
    async fn websocket_sends_snapshot_only_after_a_compatible_hello() {
        let (mut socket, _state, server) = connect_test_socket().await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome {
                protocol_version: PROTOCOL_VERSION,
                ..
            }
        ));
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "compatible-test-client".into(),
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Snapshot { .. }
        ));
        socket.close(None).await.expect("test socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn character_control_acknowledges_with_lightweight_atomic_motion_state() {
        let (mut socket, state, server) = connect_test_socket().await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome { .. }
        ));
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "motion-state-test-client".into(),
            },
        )
        .await;
        let ServerMessage::Snapshot { snapshot } = receive_server_message(&mut socket).await else {
            panic!("compatible handshake must receive a full snapshot");
        };
        let operation_id = "player-control-1-1";
        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
                operation_id: operation_id.into(),
                movement_epoch: snapshot.player.movement_epoch,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::new(0.0, 0.0, -1.0),
                angular_input: verse_protocol::Vec3::new(0.0, 0.0, 0.5),
                boost: false,
                dampeners: true,
            },
        )
        .await;
        let ServerMessage::IntentAccepted { receipt } = receive_server_message(&mut socket).await
        else {
            panic!("accepted character control must receive a receipt");
        };
        assert_eq!(receipt.operation_id, operation_id);
        let ServerMessage::MotionState { motion } = receive_server_message(&mut socket).await
        else {
            panic!("character control must publish lightweight motion state");
        };
        assert_eq!(motion.event_sequence, receipt.event_sequence);
        assert_eq!(motion.player.last_processed_input_sequence, 1);
        assert_eq!(motion.player.movement_epoch, snapshot.player.movement_epoch);
        assert_eq!(motion.grids.len(), snapshot.grids.len());
        assert_eq!(motion.world_hash, state.snapshot().world_hash);
        socket.close(None).await.expect("test socket closes");
        server.abort();
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

    #[test]
    fn life_support_progress_is_independent_of_spoofable_client_names() {
        let directory = tempdir().expect("tempdir");
        let state = AppState::new(Runtime::open(directory.path(), 99, 20).expect("runtime"));
        for _ in 0..8 {
            state.advance(250).expect("authoritative tick");
        }
        assert_eq!(state.snapshot().player.suit_oxygen_milli, 990);
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
