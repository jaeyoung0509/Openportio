use std::sync::Arc;

use axum::{extract::State, Json};
use openportio_server::api::ApiError;

use crate::{
    domain::health::{HealthResponse, ReadyResponse, StatusResponse},
    infrastructure::state::ProductionApiState,
    presentation::errors::readiness_error,
};

#[openportio_server::route(get, "/livez")]
pub(crate) async fn livez() -> Json<StatusResponse> {
    Json(StatusResponse { status: "live" })
}

#[openportio_server::route(get, "/health")]
pub(crate) async fn health(State(state): State<Arc<ProductionApiState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: state.service_name.clone(),
    })
}

#[openportio_server::route(get, "/readyz")]
pub(crate) async fn readyz(
    State(state): State<Arc<ProductionApiState>>,
) -> Result<Json<ReadyResponse>, ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .map_err(readiness_error)?;

    Ok(Json(ReadyResponse {
        status: "ready",
        database: "ok",
    }))
}
