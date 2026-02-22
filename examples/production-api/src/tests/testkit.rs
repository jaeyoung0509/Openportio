use std::sync::Arc;

use axum::Router;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use openportio_core::auth::{AudienceClaim, JwtClaims};
use openportio_server::auth::AuthRuntimeConfig;
use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::{infrastructure::state::ProductionApiState, presentation::router::build_rest_router};

pub(super) fn auth_cfg_for_tests() -> AuthRuntimeConfig {
    let mut auth_cfg = AuthRuntimeConfig::default();
    auth_cfg.enabled = true;
    auth_cfg.jwt_secret = Some("dev-secret".to_string());
    auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
    auth_cfg.expected_audience = Some("openportio-api".to_string());
    auth_cfg
}

pub(super) fn issue_test_token(secret: &str, subject: &str) -> String {
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

pub(super) fn unavailable_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://127.0.0.1:1/openportio")
        .expect("lazy pool should build")
}

pub(super) fn build_test_state() -> Arc<ProductionApiState> {
    Arc::new(ProductionApiState::new(
        "test-production-api".to_string(),
        unavailable_pool(),
    ))
}

pub(super) fn build_router_with_auth(enable_drill_routes: bool) -> Router {
    build_rest_router(
        build_test_state(),
        auth_cfg_for_tests(),
        enable_drill_routes,
    )
}

pub(super) fn build_router_without_auth() -> Router {
    build_rest_router(build_test_state(), AuthRuntimeConfig::default(), false)
}
