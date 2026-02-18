extern crate self as openportio_server;
use std::{
    convert::Infallible,
    env,
    sync::{Arc, OnceLock},
    time::Duration,
};

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Extension, Path, State},
    http::header,
    http::StatusCode,
    middleware::from_fn_with_state,
    response::{
        sse::{Event, KeepAlive, Sse},
        Html,
    },
    routing::get,
    Json, Router,
};
use openportio_core::{AppState, OpenportioError};
use openportio_rpc::{
    build_hello_response, grpc_contract_docs_markdown, grpc_contract_openapi_bridge_json,
    HelloRequest,
};
use pulldown_cmark::{html::push_html, Options, Parser};
use serde::Serialize;
use serde_json::Value;
use tokio_stream::{once, wrappers::IntervalStream, Stream, StreamExt};
use utoipa::{
    openapi::{
        path::{HttpMethod, Operation, PathItem},
        response::ResponseBuilder,
        Content, Ref,
    },
    Modify, OpenApi,
};
use utoipa_swagger_ui::SwaggerUi;

pub mod api;
pub mod auth;
pub mod builder;
pub mod di;
pub mod grpc;
pub mod middleware;
pub mod observability;
use crate::api::ApiErrorResponse;
pub use builder::OpenportioServer;
pub use openportio_macros::{dto, route};
pub use serde;
pub use utoipa;
pub use utoipa::ToSchema as MeldSchema;
pub use utoipa::ToSchema as OpenPortIOSchema;
pub use utoipa::ToSchema as OpenportioSchema;
pub use validator;
pub use validator::Validate as MeldValidate;
pub use validator::Validate as OpenPortIOValidate;
pub use validator::Validate as OpenportioValidate;
pub type MeldServer = OpenportioServer;
pub type AlloyServer = OpenportioServer;

pub mod prelude {
    pub use crate::api::{
        ApiError, ApiErrorResponse, RequestValidation, ValidatedJson, ValidatedParts,
        ValidatedPath, ValidatedQuery,
    };
    pub use crate::di::{
        with_dependency, with_dependency_override, with_dependency_overrides, DependencyOverrides,
        Depends,
    };
    pub use crate::grpc::{
        validated_grpc_request, GrpcHandlerContext, GrpcHelloRequest, GrpcHelloResponse,
        GrpcRequestValidation,
    };
    pub use crate::AlloyServer;
    pub use crate::MeldServer;
    pub use crate::OpenportioServer;
    pub use crate::{
        dto, route, MeldSchema, MeldValidate, OpenPortIOSchema, OpenPortIOValidate,
        OpenportioSchema, OpenportioValidate,
    };
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RootResponse {
    pub service_name: String,
    pub environment: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HelloRestResponse {
    pub message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProtectedWhoAmIResponse {
    pub subject: String,
    pub issuer: Option<String>,
}

#[derive(Debug, Serialize)]
struct ServerSentEventPayload {
    sequence: u64,
    kind: &'static str,
    service_name: String,
}

#[derive(OpenApi)]
#[openapi(
    paths(root, health, hello, protected_whoami),
    components(schemas(
        RootResponse,
        HealthResponse,
        HelloRestResponse,
        ProtectedWhoAmIResponse,
        ApiErrorResponse,
        crate::api::ApiValidationIssue
    )),
    modifiers(&ApiDocDefaults),
    tags(
        (name = "rest", description = "Openportio REST endpoints")
    )
)]
struct ApiDoc;

struct ApiDocDefaults;

pub fn rest_openapi_document() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

pub fn rest_openapi_json_pretty() -> String {
    serde_json::to_string_pretty(&rest_openapi_document())
        .expect("rest openapi should serialize to valid json")
}

impl Modify for ApiDocDefaults {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        add_docs_discovery_description(openapi);

        let paths = &mut openapi.paths;

        for (path, path_item) in &mut paths.paths {
            for method in [
                HttpMethod::Get,
                HttpMethod::Put,
                HttpMethod::Post,
                HttpMethod::Delete,
                HttpMethod::Patch,
                HttpMethod::Options,
                HttpMethod::Head,
                HttpMethod::Trace,
            ] {
                let Some(operation) = operation_mut(path_item, method) else {
                    continue;
                };
                ensure_error_response(
                    operation,
                    "400",
                    "Bad request or validation error",
                    "ApiErrorResponse",
                );
                ensure_error_response(
                    operation,
                    "500",
                    "Internal server error",
                    "ApiErrorResponse",
                );
                if path.starts_with("/protected/") {
                    ensure_error_response(operation, "401", "Unauthorized", "ApiErrorResponse");
                }
            }
        }
    }
}

fn add_docs_discovery_description(openapi: &mut utoipa::openapi::OpenApi) {
    let links = "Related docs: /docs (REST Swagger UI), /grpc/contracts (rendered gRPC contracts), /grpc/contracts/openapi.json (gRPC OpenAPI bridge).";
    let current = openapi.info.description.take().unwrap_or_default();
    let merged = if current.is_empty() {
        links.to_string()
    } else if current.contains("/grpc/contracts") {
        current
    } else {
        format!("{current}\n\n{links}")
    };
    openapi.info.description = Some(merged);
}

fn operation_mut(path_item: &mut PathItem, method: HttpMethod) -> Option<&mut Operation> {
    match method {
        HttpMethod::Get => path_item.get.as_mut(),
        HttpMethod::Put => path_item.put.as_mut(),
        HttpMethod::Post => path_item.post.as_mut(),
        HttpMethod::Delete => path_item.delete.as_mut(),
        HttpMethod::Options => path_item.options.as_mut(),
        HttpMethod::Head => path_item.head.as_mut(),
        HttpMethod::Patch => path_item.patch.as_mut(),
        HttpMethod::Trace => path_item.trace.as_mut(),
    }
}

fn ensure_error_response(
    operation: &mut Operation,
    status: &str,
    description: &str,
    schema_name: &str,
) {
    if operation.responses.responses.contains_key(status) {
        return;
    }

    let response = ResponseBuilder::new()
        .description(description)
        .content(
            "application/json",
            Content::new(Some(Ref::from_schema_name(schema_name))),
        )
        .build();
    operation
        .responses
        .responses
        .insert(status.to_string(), response.into());
}

pub fn build_router(state: Arc<AppState>) -> Router {
    build_router_with_auth(state, auth::AuthRuntimeConfig::from_env())
}

pub fn build_router_with_auth(state: Arc<AppState>, auth_cfg: auth::AuthRuntimeConfig) -> Router {
    let observability_cfg = observability::ObservabilityConfig::from_env();
    let protected = Router::new()
        .route("/protected/whoami", get(protected_whoami))
        .route_layer(from_fn_with_state(auth_cfg, auth::rest_auth_middleware));

    let mut router = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/hello/:name", get(hello))
        .route("/events", get(events))
        .route("/ws", get(ws_handler))
        .merge(protected)
        .route("/grpc/contracts", get(grpc_contracts))
        .route("/grpc/contracts.md", get(grpc_contracts_markdown))
        .route(
            "/grpc/contracts/openapi.json",
            get(grpc_contracts_openapi_bridge),
        )
        .merge(SwaggerUi::new("/docs").url("/openapi.json", ApiDoc::openapi()))
        .with_state(state);

    router = router.route(&observability_cfg.metrics_path, get(metrics));
    router
}

pub fn build_multiplexed_router(state: Arc<AppState>) -> Router {
    build_multiplexed_router_with_auth(state, auth::AuthRuntimeConfig::from_env())
}

pub fn build_multiplexed_router_with_auth(
    state: Arc<AppState>,
    auth_cfg: auth::AuthRuntimeConfig,
) -> Router {
    let rest = build_router_with_auth(state.clone(), auth_cfg.clone());
    let grpc = grpc::build_grpc_routes_with_auth(state, auth_cfg).into_axum_router();

    rest.merge(grpc)
}

#[utoipa::path(
    get,
    path = "/",
    tag = "rest",
    responses(
        (status = 200, description = "Root endpoint", body = RootResponse)
    )
)]
async fn root(State(state): State<Arc<AppState>>) -> Json<RootResponse> {
    Json(RootResponse {
        service_name: state.config.service_name.clone(),
        environment: state.config.environment.clone(),
    })
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "rest",
    responses(
        (status = 200, description = "Health status", body = HealthResponse)
    )
)]
async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    state.metrics.incr_counter("http.health.requests");
    Json(HealthResponse {
        status: "OK".to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/hello/{name}",
    tag = "rest",
    params(
        ("name" = String, Path, description = "Name to greet")
    ),
    responses(
        (status = 200, description = "Hello response", body = HelloRestResponse)
    )
)]
async fn hello(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<HelloRestResponse>, api::ApiError> {
    let response = build_hello_response(&state, HelloRequest { name }).map_err(map_error)?;
    Ok(Json(HelloRestResponse {
        message: response.message,
    }))
}

#[utoipa::path(
    get,
    path = "/protected/whoami",
    tag = "rest",
    responses(
        (status = 200, description = "Current principal (authenticated user or anonymous when auth is disabled)", body = ProtectedWhoAmIResponse)
    )
)]
async fn protected_whoami(
    Extension(principal): Extension<openportio_core::auth::AuthPrincipal>,
) -> Json<ProtectedWhoAmIResponse> {
    Json(ProtectedWhoAmIResponse {
        subject: principal.subject,
        issuer: principal.issuer,
    })
}

async fn events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = sse_event_stream(state.config.service_name.clone());
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}

const WS_DEFAULT_MAX_TEXT_BYTES: usize = 4 * 1024;
const WS_DEFAULT_IDLE_TIMEOUT_SECS: u64 = 45;

#[derive(Clone, Copy)]
struct WsRuntimeConfig {
    max_text_bytes: usize,
    idle_timeout: Duration,
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    let cfg = ws_runtime_config();
    ws.max_message_size(cfg.max_text_bytes)
        .on_upgrade(move |socket| handle_ws_session(socket, cfg))
}

async fn handle_ws_session(mut socket: WebSocket, cfg: WsRuntimeConfig) {
    tracing::info!("websocket connection opened");
    loop {
        let next_message = tokio::time::timeout(cfg.idle_timeout, socket.recv()).await;
        let Some(result) = (match next_message {
            Ok(result) => result,
            Err(_) => {
                tracing::info!("websocket idle timeout reached, closing connection");
                let _ = socket.close().await;
                return;
            }
        }) else {
            tracing::info!("websocket connection closed by client");
            return;
        };

        match result {
            Ok(Message::Text(text)) => {
                if text.len() > cfg.max_text_bytes {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
                if socket
                    .send(Message::Text(format!("echo: {text}")))
                    .await
                    .is_err()
                {
                    tracing::warn!("failed to send websocket text frame");
                    return;
                }
            }
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    tracing::warn!("failed to send websocket pong");
                    return;
                }
            }
            Ok(Message::Close(_)) => {
                let _ = socket.close().await;
                return;
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(error = %err, "websocket receive error");
                return;
            }
        }
    }
}

fn ws_runtime_config() -> WsRuntimeConfig {
    let max_text_bytes = read_env_with_aliases::<usize>(&[
        "OPENPORTIO_WS_MAX_TEXT_BYTES",
        "MELD_WS_MAX_TEXT_BYTES",
        "ALLOY_WS_MAX_TEXT_BYTES",
    ])
    .filter(|v| *v > 0)
    .unwrap_or(WS_DEFAULT_MAX_TEXT_BYTES);
    let idle_timeout_secs = read_env_with_aliases::<u64>(&[
        "OPENPORTIO_WS_IDLE_TIMEOUT_SECS",
        "MELD_WS_IDLE_TIMEOUT_SECS",
        "ALLOY_WS_IDLE_TIMEOUT_SECS",
    ])
    .filter(|v| *v > 0)
    .unwrap_or(WS_DEFAULT_IDLE_TIMEOUT_SECS);

    WsRuntimeConfig {
        max_text_bytes,
        idle_timeout: Duration::from_secs(idle_timeout_secs),
    }
}

fn read_env_with_aliases<T>(names: &[&str]) -> Option<T>
where
    T: std::str::FromStr,
{
    names
        .iter()
        .find_map(|name| env::var(name).ok().and_then(|raw| raw.parse::<T>().ok()))
}

async fn grpc_contracts() -> Html<&'static str> {
    Html(grpc_contracts_html_document())
}

async fn grpc_contracts_markdown() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        grpc_contract_docs_markdown(),
    )
}

async fn grpc_contracts_openapi_bridge() -> Result<Json<Value>, (StatusCode, String)> {
    serde_json::from_str(grpc_contract_openapi_bridge_json())
        .map(Json)
        .map_err(|err| {
            tracing::error!(error = %err, "failed to parse generated grpc openapi bridge json");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            )
        })
}

async fn metrics() -> ([(header::HeaderName, &'static str); 1], String) {
    (
        [(header::CONTENT_TYPE, middleware::metrics_content_type())],
        middleware::render_prometheus_metrics(),
    )
}

fn grpc_contracts_html_document() -> &'static str {
    static PAGE: OnceLock<String> = OnceLock::new();
    PAGE.get_or_init(|| {
        let mut markdown_html = String::new();
        let parser = Parser::new_ext(
            grpc_contract_docs_markdown(),
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
        );
        push_html(&mut markdown_html, parser);

        format!(
            r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Openportio gRPC Contracts</title>
    <style>
      :root {{
        color-scheme: light dark;
      }}
      body {{
        margin: 0;
        font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        line-height: 1.6;
      }}
      .container {{
        max-width: 960px;
        margin: 0 auto;
        padding: 24px;
      }}
      .links {{
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        margin-bottom: 20px;
      }}
      .links a {{
        text-decoration: none;
        font-weight: 600;
      }}
      pre {{
        overflow-x: auto;
        padding: 12px;
        border-radius: 8px;
      }}
      code {{
        font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      }}
    </style>
  </head>
  <body>
    <main class="container">
      <nav class="links">
        <a href="/grpc/contracts">Rendered gRPC Contracts</a>
        <a href="/grpc/contracts.md">Raw Markdown</a>
        <a href="/grpc/contracts/openapi.json">OpenAPI Bridge JSON</a>
        <a href="/docs">REST Swagger UI</a>
      </nav>
      {markdown_html}
    </main>
  </body>
</html>"#
        )
    })
}

fn map_error(err: OpenportioError) -> api::ApiError {
    api::map_domain_error_to_rest(err)
}

fn sse_event_stream(service_name: String) -> impl Stream<Item = Result<Event, Infallible>> {
    let init_name = service_name.clone();
    let initial = once(Ok(build_sse_event(0, "heartbeat", &init_name)));
    let mut sequence = 0u64;
    let ticks = IntervalStream::new(tokio::time::interval(Duration::from_secs(2))).map(move |_| {
        sequence += 1;
        let kind = if sequence.checked_rem(5) == Some(0) {
            "heartbeat"
        } else {
            "message"
        };
        Ok(build_sse_event(sequence, kind, &service_name))
    });

    initial.chain(ticks)
}

fn build_sse_event(sequence: u64, kind: &'static str, service_name: &str) -> Event {
    let payload = ServerSentEventPayload {
        sequence,
        kind,
        service_name: service_name.to_string(),
    };

    match Event::default()
        .id(sequence.to_string())
        .event(kind)
        .json_data(payload)
    {
        Ok(event) => event,
        Err(err) => {
            tracing::error!(error = %err, "failed to serialize sse payload");
            Event::default()
                .event("internal_error")
                .data("failed to serialize event payload")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{header::AUTHORIZATION, HeaderValue, Request, StatusCode};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::Serialize;
    use tokio::time::{timeout, Duration};
    use tokio_stream::StreamExt;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn health_returns_ok() {
        let app = build_router(Arc::new(AppState::local("test-server")));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn metrics_endpoint_is_available_with_prometheus_text() {
        let app = build_router(Arc::new(AppState::local("test-server")));
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("health request should succeed");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("metrics request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("metrics content type should exist")
            .to_str()
            .expect("metrics content type value");
        assert!(content_type.starts_with("text/plain"));

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(bytes.to_vec()).expect("valid utf8");
        assert!(body_text.contains("openportio_requests_total"));
        assert!(body_text.contains("openportio_requests_in_flight"));
        assert!(body_text.contains("openportio_request_duration_ms_total"));
    }

    #[tokio::test]
    async fn openapi_json_is_available() {
        let app = build_router(Arc::new(AppState::local("test-server")));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/openapi.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(bytes.to_vec()).expect("valid utf8");
        assert!(body_text.contains("/health"));
        assert!(body_text.contains("/hello/{name}"));
        assert!(body_text.contains("/protected/whoami"));
        assert!(body_text.contains("ApiErrorResponse"));
        assert!(body_text.contains("ApiValidationIssue"));
        assert!(body_text.contains("\"400\""));
        assert!(body_text.contains("\"500\""));
        assert!(body_text.contains("/grpc/contracts"));
    }

    #[tokio::test]
    async fn grpc_contract_docs_are_available() {
        let app = build_router(Arc::new(AppState::local("test-server")));
        let html_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/grpc/contracts")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("html request should succeed");

        assert_eq!(html_response.status(), StatusCode::OK);
        let html_content_type = html_response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("html content type should exist")
            .to_str()
            .expect("html content type value");
        assert!(html_content_type.starts_with("text/html"));

        let html_bytes = to_bytes(html_response.into_body(), usize::MAX)
            .await
            .expect("html body bytes");
        let html_text = String::from_utf8(html_bytes.to_vec()).expect("valid html");
        assert!(html_text.contains("<h1>gRPC Contract Documentation</h1>"));
        assert!(html_text.contains("/grpc/contracts.md"));
        assert!(html_text.contains("/grpc/contracts/openapi.json"));
        assert!(html_text.contains("/docs"));

        let markdown_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/grpc/contracts.md")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("markdown request should succeed");
        assert_eq!(markdown_response.status(), StatusCode::OK);
        let markdown_content_type = markdown_response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("markdown content type should exist")
            .to_str()
            .expect("markdown content type value");
        assert!(markdown_content_type.starts_with("text/markdown"));

        let markdown_bytes = to_bytes(markdown_response.into_body(), usize::MAX)
            .await
            .expect("markdown body bytes");
        let markdown_text = String::from_utf8(markdown_bytes.to_vec()).expect("valid markdown");
        assert!(markdown_text.contains("# gRPC Contract Documentation"));

        let json_response = app
            .oneshot(
                Request::builder()
                    .uri("/grpc/contracts/openapi.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("json request should succeed");
        assert_eq!(json_response.status(), StatusCode::OK);

        let json_bytes = to_bytes(json_response.into_body(), usize::MAX)
            .await
            .expect("json body bytes");
        let json_text = String::from_utf8(json_bytes.to_vec()).expect("valid json text");
        assert!(json_text.contains("/openportio.v1.Greeter/SayHello"));
    }

    #[tokio::test]
    async fn events_stream_returns_sse_headers_and_heartbeat_payload() {
        let app = build_router(Arc::new(AppState::local("test-server")));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content type should exist")
            .to_str()
            .expect("content type value");
        assert!(content_type.starts_with("text/event-stream"));

        let mut stream = response.into_body().into_data_stream();
        let first_chunk = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("first chunk should arrive")
            .expect("body stream item")
            .expect("body bytes");
        let first_text = String::from_utf8(first_chunk.to_vec()).expect("utf8 chunk");
        assert!(first_text.contains("event: heartbeat"));
        assert!(first_text.contains("\"kind\":\"heartbeat\""));
    }

    #[derive(Serialize)]
    struct TestClaims {
        sub: String,
        exp: usize,
        iss: String,
        aud: String,
    }

    fn issue_test_token(secret: &str) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            &TestClaims {
                sub: "user-123".to_string(),
                exp: 4_102_444_800,
                iss: "https://issuer.local".to_string(),
                aud: "openportio-api".to_string(),
            },
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token should encode")
    }

    #[tokio::test]
    async fn protected_route_returns_anonymous_when_auth_disabled() {
        let mut auth_cfg = auth::AuthRuntimeConfig::default();
        auth_cfg.enabled = false;
        let app = build_router_with_auth(Arc::new(AppState::local("test-server")), auth_cfg);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected/whoami")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(body_text.contains("anonymous"));
    }

    #[tokio::test]
    async fn protected_route_rejects_missing_token_when_auth_enabled() {
        let mut auth_cfg = auth::AuthRuntimeConfig::default();
        auth_cfg.enabled = true;
        auth_cfg.jwt_secret = Some("dev-secret".to_string());
        auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
        auth_cfg.expected_audience = Some("openportio-api".to_string());
        let app = build_router_with_auth(Arc::new(AppState::local("test-server")), auth_cfg);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected/whoami")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_route_accepts_valid_token_when_auth_enabled() {
        let secret = "dev-secret";
        let token = issue_test_token(secret);
        let mut auth_cfg = auth::AuthRuntimeConfig::default();
        auth_cfg.enabled = true;
        auth_cfg.jwt_secret = Some(secret.to_string());
        auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
        auth_cfg.expected_audience = Some("openportio-api".to_string());
        let app = build_router_with_auth(Arc::new(AppState::local("test-server")), auth_cfg);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/protected/whoami")
                    .header(
                        AUTHORIZATION,
                        HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let body_text = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(body_text.contains("user-123"));
    }
}
