use axum::{body::to_bytes, body::Body, http::Request, http::StatusCode};
use openportio_server::{api::ApiErrorResponse, middleware::MiddlewareConfig, OpenportioServer};
use tower::util::ServiceExt;

use crate::tests::testkit::{build_router_with_auth, build_router_without_auth, issue_test_token};

#[tokio::test]
async fn readyz_returns_503_when_database_is_unavailable() {
    let app = build_router_without_auth();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn notes_routes_require_auth_when_enabled() {
    let app = build_router_with_auth(false);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let parsed: ApiErrorResponse = serde_json::from_slice(&body).expect("error body should parse");
    assert_eq!(parsed.code, "unauthorized");
}

#[tokio::test]
async fn notes_route_rejects_invalid_bearer_token() {
    let app = build_router_with_auth(false);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes")
                .header("authorization", "Bearer invalid-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let parsed: ApiErrorResponse = serde_json::from_slice(&body).expect("error body should parse");
    assert_eq!(parsed.code, "unauthorized");
}

#[tokio::test]
async fn notes_route_returns_500_when_database_is_unavailable() {
    let app = build_router_with_auth(false);
    let token = issue_test_token("dev-secret", "test-user");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes?limit=5")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let parsed: ApiErrorResponse = serde_json::from_slice(&body).expect("error body should parse");
    assert_eq!(parsed.code, "internal_error");
}

#[tokio::test]
async fn notes_route_validation_failure_returns_400_before_database_access() {
    let app = build_router_with_auth(false);
    let token = issue_test_token("dev-secret", "test-user");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/notes?limit=101")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let parsed: ApiErrorResponse = serde_json::from_slice(&body).expect("error body should parse");
    assert_eq!(parsed.code, "validation_error");
}

#[tokio::test]
async fn livez_stays_ok_when_readyz_is_degraded() {
    let app = build_router_without_auth();

    let readyz = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("readyz request should complete");
    assert_eq!(readyz.status(), StatusCode::SERVICE_UNAVAILABLE);

    let livez = app
        .oneshot(
            Request::builder()
                .uri("/livez")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("livez request should complete");
    assert_eq!(livez.status(), StatusCode::OK);
}

#[tokio::test]
async fn drill_route_times_out_when_timeout_budget_is_exceeded() {
    let rest_router = build_router_with_auth(true);
    let app = OpenportioServer::new()
        .without_grpc()
        .with_rest_router(rest_router)
        .with_middleware_config(MiddlewareConfig {
            timeout_seconds: 1,
            ..MiddlewareConfig::default()
        })
        .build_app();

    let token = issue_test_token("dev-secret", "test-user");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/ops/drill/sleep/2")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let message = String::from_utf8(body.to_vec()).expect("timeout body should be utf8");
    assert!(message.contains("request timed out"));
}
