use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Extension, FromRef, FromRequestParts, State},
    http::request::Parts,
    middleware::from_fn_with_state,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use openportio_core::{auth::AuthPrincipal, AppState, OpenportioError};
use openportio_server::{
    api::{bad_request, ApiError, ValidatedJson, ValidatedPath, ValidatedQuery},
    auth::{self, AuthRuntimeConfig},
    di::Depends,
    grpc::{validated_grpc_request, GrpcHandlerContext, GrpcHelloRequest, GrpcHelloResponse},
    OpenportioServer,
};
use serde::{Deserialize, Serialize};
use tokio_stream::{once, wrappers::IntervalStream, Stream, StreamExt};

#[openportio_server::dto]
struct NotePath {
    #[validate(length(min = 3))]
    id: String,
}

#[openportio_server::dto]
struct NoteQuery {
    #[validate(length(max = 80))]
    q: Option<String>,
    #[validate(range(min = 1, max = 100))]
    limit: Option<u32>,
}

#[openportio_server::dto]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}

#[openportio_server::dto]
struct GrpcSayHelloInput {
    #[validate(length(min = 1, max = 80))]
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct NoteResponse {
    id: String,
    title: String,
    request_id: Option<String>,
    service_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct NotesListResponse {
    query: Option<String>,
    limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProtectedGreetingResponse {
    subject: String,
    message: String,
    service_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct NoteEventPayload {
    sequence: u64,
    kind: String,
    message: String,
}

#[derive(Debug, Clone)]
struct RequestContext {
    request_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ServiceInfo {
    service_name: String,
}

impl FromRef<Arc<AppState>> for ServiceInfo {
    fn from_ref(state: &Arc<AppState>) -> Self {
        Self {
            service_name: state.config.service_name.clone(),
        }
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for RequestContext
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        Ok(Self { request_id })
    }
}

fn execute_shared_greeting_use_case(
    state: &Arc<AppState>,
    service_name: &str,
    actor: &str,
    name: &str,
    protocol: &'static str,
) -> Result<String, OpenportioError> {
    let greeting = state.greet(name)?;
    Ok(format!("[{service_name}:{protocol}:{actor}] {greeting}"))
}

#[openportio_server::route(get, "/notes/:id", auto_validate, transparent)]
async fn get_note(
    ctx: RequestContext,
    Depends(service): Depends<ServiceInfo>,
    State(state): State<Arc<AppState>>,
    ValidatedPath(path): ValidatedPath<NotePath>,
) -> Result<Json<NoteResponse>, ApiError> {
    let title = state
        .greet(&path.id)
        .map_err(|err| bad_request(err.to_string()))?;
    Ok(Json(NoteResponse {
        id: path.id,
        title,
        request_id: ctx.request_id,
        service_name: service.service_name,
    }))
}

#[openportio_server::route(get, "/protected/greet/:id", auto_validate, transparent)]
async fn get_protected_greet(
    Extension(principal): Extension<AuthPrincipal>,
    Depends(service): Depends<ServiceInfo>,
    State(state): State<Arc<AppState>>,
    ValidatedPath(path): ValidatedPath<NotePath>,
) -> Result<Json<ProtectedGreetingResponse>, ApiError> {
    let message = execute_shared_greeting_use_case(
        &state,
        &service.service_name,
        &principal.subject,
        &path.id,
        "rest",
    )
    .map_err(|err| bad_request(err.to_string()))?;

    Ok(Json(ProtectedGreetingResponse {
        subject: principal.subject,
        message,
        service_name: service.service_name,
    }))
}

#[openportio_server::route(get, "/notes", auto_validate, transparent)]
async fn list_notes(ValidatedQuery(query): ValidatedQuery<NoteQuery>) -> Json<NotesListResponse> {
    Json(NotesListResponse {
        query: query.q,
        limit: query.limit.unwrap_or(20),
    })
}

#[openportio_server::route(post, "/notes", auto_validate, transparent)]
async fn create_note(
    ctx: RequestContext,
    Depends(service): Depends<ServiceInfo>,
    ValidatedJson(body): ValidatedJson<CreateNoteBody>,
) -> Json<NoteResponse> {
    Json(NoteResponse {
        id: "note-1".to_string(),
        title: body.title,
        request_id: ctx.request_id,
        service_name: service.service_name,
    })
}

#[openportio_server::route(post, "/notes/raw")]
async fn create_note_raw(Json(body): Json<CreateNoteBody>) -> Json<NoteResponse> {
    Json(NoteResponse {
        id: "note-raw".to_string(),
        title: body.title,
        request_id: None,
        service_name: "raw".to_string(),
    })
}

async fn stream_note_events() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(note_event_stream()).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}

fn note_event_stream() -> impl Stream<Item = Result<Event, Infallible>> {
    let initial = once(Ok(note_event(0, "heartbeat")));
    let mut sequence = 0u64;
    let ticks = IntervalStream::new(tokio::time::interval(Duration::from_secs(2))).map(move |_| {
        sequence += 1;
        let kind = if sequence.checked_rem(5) == Some(0) {
            "heartbeat"
        } else {
            "note"
        };
        Ok(note_event(sequence, kind))
    });
    initial.chain(ticks)
}

fn note_event(sequence: u64, kind: &str) -> Event {
    let payload = NoteEventPayload {
        sequence,
        kind: kind.to_string(),
        message: format!("note event #{sequence}"),
    };

    match Event::default()
        .id(sequence.to_string())
        .event(kind)
        .json_data(payload)
    {
        Ok(event) => event,
        Err(err) => {
            eprintln!("failed to serialize note event payload: {err}");
            Event::default()
                .event("internal_error")
                .data("failed to serialize note event payload")
        }
    }
}

async fn grpc_say_hello(
    ctx: GrpcHandlerContext,
    request: GrpcHelloRequest,
) -> Result<GrpcHelloResponse, OpenportioError> {
    let input = validated_grpc_request(GrpcSayHelloInput { name: request.name })?;
    let service = ctx.depends::<ServiceInfo>();
    let message = execute_shared_greeting_use_case(
        &ctx.state(),
        &service.service_name,
        &ctx.principal().subject,
        &input.name,
        "grpc",
    )?;
    Ok(GrpcHelloResponse { message })
}

const WS_MAX_TEXT_BYTES: usize = 4 * 1024;
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

async fn ws_echo(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.max_message_size(WS_MAX_TEXT_BYTES)
        .on_upgrade(handle_ws_echo_session)
}

async fn handle_ws_echo_session(mut socket: WebSocket) {
    loop {
        let next_message = tokio::time::timeout(WS_IDLE_TIMEOUT, socket.recv()).await;
        let Some(result) = (match next_message {
            Ok(result) => result,
            Err(_) => {
                let _ = socket.close().await;
                return;
            }
        }) else {
            return;
        };

        match result {
            Ok(Message::Text(text)) => {
                if text.len() > WS_MAX_TEXT_BYTES {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
                if socket
                    .send(Message::Text(format!("echo: {text}")))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    return;
                }
            }
            Ok(Message::Close(_)) => {
                let _ = socket.close().await;
                return;
            }
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

fn build_protected_router(auth_cfg: AuthRuntimeConfig) -> Router<Arc<AppState>> {
    Router::new()
        .route("/protected/greet/:id", get(get_protected_greet))
        .route_layer(from_fn_with_state(auth_cfg, auth::rest_auth_middleware))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState::local("simple-server"));
    let auth_cfg = AuthRuntimeConfig::from_env();
    let custom_router = Router::new()
        .route("/notes", get(list_notes).post(create_note))
        .route("/notes/raw", axum::routing::post(create_note_raw))
        .route("/events", get(stream_note_events))
        .route("/ws", get(ws_echo))
        .route("/notes/:id", get(get_note))
        .merge(build_protected_router(auth_cfg))
        .with_state(state.clone());

    OpenportioServer::new()
        .with_state(state)
        .with_grpc_say_hello_with_context(grpc_say_hello)
        .with_rest_router(custom_router)
        .with_addr(SocketAddr::from(([127, 0, 0, 1], 4000)))
        .on_startup(|addr| {
            println!("simple-server started on {addr}");
        })
        .run()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use futures_util::{SinkExt, StreamExt as FuturesStreamExt};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use openportio_core::auth::{AudienceClaim, JwtClaims};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tower::util::ServiceExt;

    fn app() -> Router {
        let state = Arc::new(AppState::local("simple-server-test"));
        Router::new()
            .route("/notes", get(list_notes).post(create_note))
            .route("/notes/raw", axum::routing::post(create_note_raw))
            .route("/events", get(stream_note_events))
            .route("/ws", get(ws_echo))
            .route("/notes/:id", get(get_note))
            .merge(build_protected_router(AuthRuntimeConfig::default()))
            .with_state(state)
    }

    fn app_with_auth_enabled() -> Router {
        let state = Arc::new(AppState::local("simple-server-test"));
        let mut auth_cfg = AuthRuntimeConfig::default();
        auth_cfg.enabled = true;
        auth_cfg.jwt_secret = Some("dev-secret".to_string());
        auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
        auth_cfg.expected_audience = Some("openportio-api".to_string());

        Router::new()
            .route("/notes", get(list_notes).post(create_note))
            .route("/notes/raw", axum::routing::post(create_note_raw))
            .route("/events", get(stream_note_events))
            .route("/ws", get(ws_echo))
            .route("/notes/:id", get(get_note))
            .merge(build_protected_router(auth_cfg))
            .with_state(state)
    }

    fn issue_test_token(secret: &str, subject: &str) -> String {
        let claims = JwtClaims {
            sub: subject.to_string(),
            exp: 4_102_444_800,
            iss: Some("https://issuer.local".to_string()),
            aud: Some(AudienceClaim::One("openportio-api".to_string())),
            scope: Some("read:notes write:notes".to_string()),
        };

        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token should encode")
    }

    #[tokio::test]
    async fn invalid_body_returns_structured_400() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/notes")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"title":"x"}"#))
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: openportio_server::api::ApiErrorResponse =
            serde_json::from_slice(&body).expect("api error json");
        assert_eq!(parsed.code, "validation_error");
        let detail = parsed.detail.expect("validation detail should exist");
        assert!(detail
            .iter()
            .any(|issue| issue.loc.first() == Some(&"body".to_string())));
    }

    #[tokio::test]
    async fn valid_body_returns_note() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/notes")
                    .header("content-type", "application/json")
                    .header("x-request-id", "req-1")
                    .body(axum::body::Body::from(r#"{"title":"My Note"}"#))
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: NoteResponse = serde_json::from_slice(&body).expect("note json");
        assert_eq!(parsed.title, "My Note");
        assert_eq!(parsed.request_id.as_deref(), Some("req-1"));
        assert_eq!(parsed.service_name, "simple-server-test");
    }

    #[tokio::test]
    async fn invalid_query_returns_structured_400() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/notes?limit=0")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: openportio_server::api::ApiErrorResponse =
            serde_json::from_slice(&body).expect("api error json");
        assert_eq!(parsed.code, "validation_error");
        let detail = parsed.detail.expect("validation detail should exist");
        assert!(detail
            .iter()
            .any(|issue| issue.loc.first() == Some(&"query".to_string())));
    }

    #[tokio::test]
    async fn invalid_path_returns_structured_400() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/notes/ab")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: openportio_server::api::ApiErrorResponse =
            serde_json::from_slice(&body).expect("api error json");
        assert_eq!(parsed.code, "validation_error");
        let detail = parsed.detail.expect("validation detail should exist");
        assert!(detail
            .iter()
            .any(|issue| issue.loc.first() == Some(&"path".to_string())));
    }

    #[tokio::test]
    async fn protected_route_requires_auth_when_enabled() {
        let response = app_with_auth_enabled()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/protected/greet/rust")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_route_succeeds_with_valid_token() {
        let token = issue_test_token("dev-secret", "user-1");

        let response = app_with_auth_enabled()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/protected/greet/rust")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ProtectedGreetingResponse =
            serde_json::from_slice(&body).expect("protected route payload");
        assert_eq!(parsed.subject, "user-1");
        assert!(parsed.message.contains("[simple-server-test:rest:user-1]"));
    }

    #[test]
    fn shared_greeting_use_case_is_protocol_agnostic() {
        let state = Arc::new(AppState::local("simple-server-test"));
        let rest_message = execute_shared_greeting_use_case(
            &state,
            "simple-server-test",
            "rest-user",
            "Rust",
            "rest",
        )
        .expect("rest message");
        let grpc_message = execute_shared_greeting_use_case(
            &state,
            "simple-server-test",
            "grpc-user",
            "Rust",
            "grpc",
        )
        .expect("grpc message");

        assert!(rest_message.contains("[simple-server-test:rest:rest-user]"));
        assert!(grpc_message.contains("[simple-server-test:grpc:grpc-user]"));
    }

    #[tokio::test]
    async fn without_auto_validate_keeps_original_behavior() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/notes/raw")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"title":"x"}"#))
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn events_route_streams_heartbeat_payload() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/events")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .expect("content-type should exist")
            .to_str()
            .expect("content-type should be valid");
        assert!(content_type.starts_with("text/event-stream"));

        let mut stream = response.into_body().into_data_stream();
        let first_chunk = timeout(
            Duration::from_secs(1),
            tokio_stream::StreamExt::next(&mut stream),
        )
        .await
        .expect("first chunk should arrive")
        .expect("stream item")
        .expect("body bytes");
        let first_text = String::from_utf8(first_chunk.to_vec()).expect("utf8 chunk");
        assert!(first_text.contains("event: heartbeat"));
        assert!(first_text.contains("\"kind\":\"heartbeat\""));
    }

    #[tokio::test]
    async fn ws_route_handshake_and_echo() {
        let app = app();
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("server should run");
        });

        let ws_url = format!("ws://{addr}/ws");
        let (mut ws_stream, ws_resp) = tokio_tungstenite::connect_async(ws_url)
            .await
            .expect("websocket handshake should succeed");
        assert_eq!(ws_resp.status().as_u16(), 101);

        ws_stream
            .send(WsMessage::Text("hello-note".to_string()))
            .await
            .expect("ws send should succeed");
        let ws_message = FuturesStreamExt::next(&mut ws_stream)
            .await
            .expect("ws message item should exist")
            .expect("ws message should be valid");
        match ws_message {
            WsMessage::Text(text) => assert_eq!(text, "echo: hello-note"),
            other => panic!("expected text frame, got {other:?}"),
        }
        ws_stream
            .close(None)
            .await
            .expect("ws close should succeed");

        let _ = shutdown_tx.send(());
        let _ = server.await;
    }
}
