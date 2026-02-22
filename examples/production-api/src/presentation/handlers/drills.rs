use std::time::Duration;

use axum::Json;

use crate::{domain::health::StatusResponse, presentation::dto::DrillSleepPath};
use openportio_server::api::ValidatedPath;

#[openportio_server::route(get, "/ops/drill/sleep/:seconds", auto_validate, transparent)]
pub(crate) async fn drill_sleep(
    ValidatedPath(path): ValidatedPath<DrillSleepPath>,
) -> Json<StatusResponse> {
    tokio::time::sleep(Duration::from_secs(path.seconds)).await;
    Json(StatusResponse {
        status: "completed",
    })
}
