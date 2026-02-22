use std::sync::Arc;

use axum::{middleware::from_fn_with_state, routing::get, Router};
use openportio_server::{
    auth::{self, AuthRuntimeConfig},
    observability,
};

use crate::{
    infrastructure::state::ProductionApiState,
    presentation::handlers::{
        drills::drill_sleep,
        greeting::greet,
        health::{health, livez, readyz},
        notes::{create_note, get_protected_note, list_notes},
        observability::metrics,
    },
};

pub(crate) fn build_rest_router(
    state: Arc<ProductionApiState>,
    auth_cfg: AuthRuntimeConfig,
    enable_drill_routes: bool,
) -> Router {
    let observability_cfg = observability::ObservabilityConfig::from_env();
    let mut protected_router = Router::new()
        .route("/v1/greetings/:name", get(greet))
        .route("/v1/notes", get(list_notes).post(create_note))
        .route("/protected/notes/:id", get(get_protected_note));

    if enable_drill_routes {
        protected_router = protected_router.route("/ops/drill/sleep/:seconds", get(drill_sleep));
    }

    let notes_router =
        protected_router.route_layer(from_fn_with_state(auth_cfg, auth::rest_auth_middleware));

    Router::new()
        .route("/livez", get(livez))
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route(&observability_cfg.metrics_path, get(metrics))
        .merge(notes_router)
        .with_state(state)
}
