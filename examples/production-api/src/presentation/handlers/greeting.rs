use std::sync::Arc;

use axum::{
    extract::{Extension, State},
    Json,
};
use openportio_core::auth::AuthPrincipal;
use openportio_server::api::{self, ApiError, ValidatedPath};

use crate::{
    domain::greeting::{GreetingChannel, GreetingCommand, GreetingResponse},
    infrastructure::state::ProductionApiState,
    presentation::dto::GreetingPath,
};

#[openportio_server::route(get, "/v1/greetings/:name", auto_validate, transparent)]
pub(crate) async fn greet(
    Extension(principal): Extension<AuthPrincipal>,
    State(state): State<Arc<ProductionApiState>>,
    ValidatedPath(path): ValidatedPath<GreetingPath>,
) -> Result<Json<GreetingResponse>, ApiError> {
    let command = GreetingCommand {
        name: path.name,
        actor: Some(principal.subject),
        channel: GreetingChannel::Rest,
    };
    let result = state
        .greeting_use_case
        .execute(command)
        .map_err(api::map_domain_error_to_rest)?;

    Ok(Json(result.into_response()))
}
