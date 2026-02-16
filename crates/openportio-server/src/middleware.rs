use std::{
    env,
    str::FromStr,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

use axum::{
    error_handling::HandleErrorLayer,
    extract::Request,
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{from_fn, Next},
    response::Response,
    BoxError, Router,
};
use tower::{limit::ConcurrencyLimitLayer, timeout::TimeoutLayer, ServiceBuilder};
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

const REQUEST_ID_HEADER: &str = "x-request-id";
const DEFAULT_TIMEOUT_SECONDS: u64 = 15;
const DEFAULT_MAX_IN_FLIGHT_REQUESTS: usize = 1024;
const DEFAULT_REQUEST_BODY_LIMIT_BYTES: usize = 1_048_576;
const METRICS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Debug, Default)]
struct MetricsState {
    requests_total: AtomicU64,
    requests_in_flight: AtomicU64,
    requests_2xx_total: AtomicU64,
    requests_4xx_total: AtomicU64,
    requests_5xx_total: AtomicU64,
    request_duration_ms_total: AtomicU64,
}

#[derive(Debug, Clone, Copy)]
pub struct MetricsSnapshot {
    pub requests_total: u64,
    pub requests_in_flight: u64,
    pub requests_2xx_total: u64,
    pub requests_4xx_total: u64,
    pub requests_5xx_total: u64,
    pub request_duration_ms_total: u64,
}

static METRICS: OnceLock<MetricsState> = OnceLock::new();

#[derive(Debug, Clone, Default)]
pub enum CorsAllowOrigins {
    #[default]
    None,
    Any,
    List(Vec<HeaderValue>),
}

#[derive(Debug, Clone)]
pub struct MiddlewareConfig {
    pub timeout_seconds: u64,
    pub max_in_flight_requests: usize,
    pub max_request_body_bytes: usize,
    pub cors_allow_origins: CorsAllowOrigins,
}

impl Default for MiddlewareConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            max_in_flight_requests: DEFAULT_MAX_IN_FLIGHT_REQUESTS,
            max_request_body_bytes: DEFAULT_REQUEST_BODY_LIMIT_BYTES,
            cors_allow_origins: CorsAllowOrigins::None,
        }
    }
}

impl MiddlewareConfig {
    pub fn from_env() -> Self {
        Self {
            timeout_seconds: read_env_with_aliases(&[
                "OPENPORTIO_TIMEOUT_SECONDS",
                "MELD_TIMEOUT_SECONDS",
                "ALLOY_TIMEOUT_SECONDS",
            ])
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            max_in_flight_requests: read_env_with_aliases(&[
                "OPENPORTIO_MAX_IN_FLIGHT_REQUESTS",
                "MELD_MAX_IN_FLIGHT_REQUESTS",
                "ALLOY_MAX_IN_FLIGHT_REQUESTS",
            ])
            .unwrap_or(DEFAULT_MAX_IN_FLIGHT_REQUESTS),
            max_request_body_bytes: read_env_with_aliases(&[
                "OPENPORTIO_REQUEST_BODY_LIMIT_BYTES",
                "MELD_REQUEST_BODY_LIMIT_BYTES",
                "ALLOY_REQUEST_BODY_LIMIT_BYTES",
            ])
            .unwrap_or(DEFAULT_REQUEST_BODY_LIMIT_BYTES),
            cors_allow_origins: parse_cors_allow_origins(read_env_string_with_aliases(&[
                "OPENPORTIO_CORS_ALLOW_ORIGINS",
                "MELD_CORS_ALLOW_ORIGINS",
                "ALLOY_CORS_ALLOW_ORIGINS",
            ])),
        }
    }
}

pub fn metrics_snapshot() -> MetricsSnapshot {
    let metrics = metrics_state();
    MetricsSnapshot {
        requests_total: metrics.requests_total.load(Ordering::Relaxed),
        requests_in_flight: metrics.requests_in_flight.load(Ordering::Relaxed),
        requests_2xx_total: metrics.requests_2xx_total.load(Ordering::Relaxed),
        requests_4xx_total: metrics.requests_4xx_total.load(Ordering::Relaxed),
        requests_5xx_total: metrics.requests_5xx_total.load(Ordering::Relaxed),
        request_duration_ms_total: metrics.request_duration_ms_total.load(Ordering::Relaxed),
    }
}

pub fn render_prometheus_metrics() -> String {
    let snapshot = metrics_snapshot();
    format!(
        "# HELP openportio_requests_total Total number of HTTP requests seen by shared middleware.\n\
# TYPE openportio_requests_total counter\n\
openportio_requests_total {}\n\
# HELP openportio_requests_in_flight Current number of in-flight HTTP requests.\n\
# TYPE openportio_requests_in_flight gauge\n\
openportio_requests_in_flight {}\n\
# HELP openportio_requests_2xx_total Total number of responses with 2xx status.\n\
# TYPE openportio_requests_2xx_total counter\n\
openportio_requests_2xx_total {}\n\
# HELP openportio_requests_4xx_total Total number of responses with 4xx status.\n\
# TYPE openportio_requests_4xx_total counter\n\
openportio_requests_4xx_total {}\n\
# HELP openportio_requests_5xx_total Total number of responses with 5xx status.\n\
# TYPE openportio_requests_5xx_total counter\n\
openportio_requests_5xx_total {}\n\
# HELP openportio_request_duration_ms_total Total request duration in milliseconds.\n\
# TYPE openportio_request_duration_ms_total counter\n\
openportio_request_duration_ms_total {}\n",
        snapshot.requests_total,
        snapshot.requests_in_flight,
        snapshot.requests_2xx_total,
        snapshot.requests_4xx_total,
        snapshot.requests_5xx_total,
        snapshot.request_duration_ms_total,
    )
}

pub fn metrics_content_type() -> &'static str {
    METRICS_CONTENT_TYPE
}

pub fn apply_shared_middleware(app: Router, config: &MiddlewareConfig) -> Router {
    let app = match &config.cors_allow_origins {
        CorsAllowOrigins::None => app,
        CorsAllowOrigins::Any => app.layer(CorsLayer::new().allow_origin(Any)),
        CorsAllowOrigins::List(origins) => {
            app.layer(CorsLayer::new().allow_origin(origins.clone()))
        }
    };

    let app = app.layer(
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(handle_middleware_error))
            .layer(TraceLayer::new_for_http())
            .layer(SetRequestIdLayer::new(header_name(), MakeRequestUuid))
            .layer(PropagateRequestIdLayer::new(header_name()))
            .layer(RequestBodyLimitLayer::new(config.max_request_body_bytes))
            .layer(TimeoutLayer::new(Duration::from_secs(
                config.timeout_seconds,
            )))
            .layer(ConcurrencyLimitLayer::new(config.max_in_flight_requests)),
    );

    app.layer(from_fn(observe_request_metrics))
}

fn metrics_state() -> &'static MetricsState {
    METRICS.get_or_init(MetricsState::default)
}

#[cfg(test)]
fn reset_metrics_for_tests() {
    if let Some(metrics) = METRICS.get() {
        metrics.requests_total.store(0, Ordering::Relaxed);
        metrics.requests_in_flight.store(0, Ordering::Relaxed);
        metrics.requests_2xx_total.store(0, Ordering::Relaxed);
        metrics.requests_4xx_total.store(0, Ordering::Relaxed);
        metrics.requests_5xx_total.store(0, Ordering::Relaxed);
        metrics
            .request_duration_ms_total
            .store(0, Ordering::Relaxed);
    }
}

async fn observe_request_metrics(request: Request, next: Next) -> Response {
    let metrics = metrics_state();
    metrics.requests_in_flight.fetch_add(1, Ordering::Relaxed);
    let started_at = Instant::now();

    let response = next.run(request).await;

    metrics.requests_total.fetch_add(1, Ordering::Relaxed);
    metrics.requests_in_flight.fetch_sub(1, Ordering::Relaxed);
    let duration_ms = started_at.elapsed().as_millis();
    let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
    metrics
        .request_duration_ms_total
        .fetch_add(duration_ms, Ordering::Relaxed);

    let status = response.status();
    if status.is_server_error() {
        metrics.requests_5xx_total.fetch_add(1, Ordering::Relaxed);
    } else if status.is_client_error() {
        metrics.requests_4xx_total.fetch_add(1, Ordering::Relaxed);
    } else if status.is_success() {
        metrics.requests_2xx_total.fetch_add(1, Ordering::Relaxed);
    }

    response
}

async fn handle_middleware_error(error: BoxError) -> (StatusCode, String) {
    if error.is::<tower::timeout::error::Elapsed>() {
        return (StatusCode::REQUEST_TIMEOUT, "request timed out".to_string());
    }

    tracing::error!(error = %error, "unhandled middleware error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".to_string(),
    )
}

fn header_name() -> HeaderName {
    HeaderName::from_static(REQUEST_ID_HEADER)
}

fn read_env<T>(name: &str) -> Option<T>
where
    T: FromStr,
{
    env::var(name).ok().and_then(|raw| raw.parse::<T>().ok())
}

fn read_env_with_aliases<T>(names: &[&str]) -> Option<T>
where
    T: FromStr,
{
    names.iter().find_map(|name| read_env(name))
}

fn read_env_string_with_aliases(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| env::var(name).ok())
}

fn parse_cors_allow_origins(raw: Option<String>) -> CorsAllowOrigins {
    let Some(raw) = raw else {
        return CorsAllowOrigins::None;
    };

    let raw = raw.trim();
    if raw.is_empty() {
        return CorsAllowOrigins::None;
    }
    if raw == "*" {
        return CorsAllowOrigins::Any;
    }

    let origins = raw
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .filter_map(|origin| match HeaderValue::from_str(origin) {
            Ok(value) => Some(value),
            Err(err) => {
                tracing::warn!(origin = %origin, error = %err, "ignoring invalid cors origin");
                None
            }
        })
        .collect::<Vec<_>>();

    if origins.is_empty() {
        CorsAllowOrigins::None
    } else {
        CorsAllowOrigins::List(origins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header::ORIGIN, Request},
        routing::{get, post},
    };
    use std::sync::{LazyLock, Mutex};
    use tower::util::ServiceExt;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn default_config_is_reasonable() {
        let config = MiddlewareConfig::default();
        assert_eq!(config.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
        assert_eq!(
            config.max_in_flight_requests,
            DEFAULT_MAX_IN_FLIGHT_REQUESTS
        );
        assert_eq!(
            config.max_request_body_bytes,
            DEFAULT_REQUEST_BODY_LIMIT_BYTES
        );
        assert!(matches!(config.cors_allow_origins, CorsAllowOrigins::None));
    }

    #[test]
    fn parse_cors_allow_origins_supports_wildcard_and_allowlist() {
        assert!(matches!(
            parse_cors_allow_origins(Some("*".to_string())),
            CorsAllowOrigins::Any
        ));

        let parsed =
            parse_cors_allow_origins(Some("https://one.example, https://two.example".to_string()));
        match parsed {
            CorsAllowOrigins::List(origins) => assert_eq!(origins.len(), 2),
            _ => panic!("expected allowlist"),
        }
    }

    #[tokio::test]
    async fn default_cors_policy_does_not_emit_allow_origin_header() {
        let app = apply_shared_middleware(
            Router::new().route("/health", get(|| async { "ok" })),
            &MiddlewareConfig::default(),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header(ORIGIN, "https://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
    }

    #[tokio::test]
    async fn request_body_limit_rejects_oversized_payload() {
        let config = MiddlewareConfig {
            max_request_body_bytes: 8,
            ..MiddlewareConfig::default()
        };

        let app = apply_shared_middleware(
            Router::new().route("/echo", post(|body: String| async move { body })),
            &config,
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/echo")
                    .body(Body::from("0123456789012345"))
                    .unwrap(),
            )
            .await
            .expect("request should complete");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn request_id_is_generated_and_propagated() {
        let app = apply_shared_middleware(
            Router::new().route("/health", get(|| async { "ok" })),
            &MiddlewareConfig::default(),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert!(response.headers().get(REQUEST_ID_HEADER).is_some());
    }

    #[tokio::test]
    async fn metrics_are_recorded_by_shared_middleware() {
        reset_metrics_for_tests();
        let before = metrics_snapshot();
        let app = apply_shared_middleware(
            Router::new().route("/health", get(|| async { "ok" })),
            &MiddlewareConfig::default(),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");
        assert_eq!(response.status(), StatusCode::OK);

        let snapshot = metrics_snapshot();
        assert!(snapshot.requests_total > before.requests_total);
        assert!(snapshot.requests_2xx_total > before.requests_2xx_total);
        assert!(
            snapshot.request_duration_ms_total >= before.request_duration_ms_total,
            "duration counter should be monotonic"
        );
    }

    #[tokio::test]
    async fn middleware_internal_error_response_is_generic() {
        let (status, body) =
            handle_middleware_error(std::io::Error::other("sensitive").into()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, "internal server error");
    }

    #[test]
    fn from_env_supports_meld_compatibility_aliases() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        clear_middleware_env();

        env::set_var("MELD_TIMEOUT_SECONDS", "9");
        env::set_var("MELD_MAX_IN_FLIGHT_REQUESTS", "77");
        env::set_var("MELD_REQUEST_BODY_LIMIT_BYTES", "4096");
        env::set_var("MELD_CORS_ALLOW_ORIGINS", "https://legacy.example");

        let cfg = MiddlewareConfig::from_env();
        assert_eq!(cfg.timeout_seconds, 9);
        assert_eq!(cfg.max_in_flight_requests, 77);
        assert_eq!(cfg.max_request_body_bytes, 4096);
        match cfg.cors_allow_origins {
            CorsAllowOrigins::List(origins) => assert_eq!(origins.len(), 1),
            _ => panic!("expected list cors config"),
        }

        clear_middleware_env();
    }

    fn clear_middleware_env() {
        for key in [
            "OPENPORTIO_TIMEOUT_SECONDS",
            "OPENPORTIO_MAX_IN_FLIGHT_REQUESTS",
            "OPENPORTIO_REQUEST_BODY_LIMIT_BYTES",
            "OPENPORTIO_CORS_ALLOW_ORIGINS",
            "MELD_TIMEOUT_SECONDS",
            "MELD_MAX_IN_FLIGHT_REQUESTS",
            "MELD_REQUEST_BODY_LIMIT_BYTES",
            "MELD_CORS_ALLOW_ORIGINS",
            "ALLOY_TIMEOUT_SECONDS",
            "ALLOY_MAX_IN_FLIGHT_REQUESTS",
            "ALLOY_REQUEST_BODY_LIMIT_BYTES",
            "ALLOY_CORS_ALLOW_ORIGINS",
        ] {
            env::remove_var(key);
        }
    }
}
