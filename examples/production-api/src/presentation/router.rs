use std::sync::Arc;

use axum::{middleware::from_fn_with_state, routing::get, Router};
use openportio_server::auth::{self, AuthRuntimeConfig};

use crate::{
    infrastructure::state::ProductionApiState,
    presentation::handlers::{
        drills::drill_sleep,
        health::{health, livez, readyz},
        notes::{create_note, get_protected_note, list_notes},
    },
};

pub(crate) fn build_rest_router(
    state: Arc<ProductionApiState>,
    auth_cfg: AuthRuntimeConfig,
    enable_drill_routes: bool,
) -> Router {
    let mut protected_router = Router::new()
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
        .merge(notes_router)
        .with_state(state)
}
