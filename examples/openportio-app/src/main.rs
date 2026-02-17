use axum::{Json, Router};
use openportio::api::{ValidatedJson, ValidatedPath};
use openportio::prelude::*;

#[openportio::dto]
struct Payload {
    #[validate(length(min = 1, max = 40))]
    name: String,
}

#[openportio::dto]
struct ItemPath {
    #[validate(length(min = 3))]
    id: String,
}

#[openportio::route(post, "/payload", auto_validate, transparent)]
async fn create_payload(
    ValidatedJson(payload): ValidatedJson<Payload>,
) -> Result<Json<String>, ApiError> {
    Ok(Json(payload.name))
}

#[openportio::route(get, "/items/:id", auto_validate, transparent)]
async fn get_item(ValidatedPath(path): ValidatedPath<ItemPath>) -> Result<Json<String>, ApiError> {
    Ok(Json(path.id))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let router = Router::new()
        .route("/payload", axum::routing::post(create_payload))
        .route("/items/:id", axum::routing::get(get_item));

    OpenportioServer::new()
        .with_rest_router(router)
        .without_grpc()
        .run()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use tower::util::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/payload", axum::routing::post(create_payload))
            .route("/items/:id", axum::routing::get(get_item))
    }

    #[tokio::test]
    async fn dependency_macro_validates_path() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/items/ab")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ApiErrorResponse = serde_json::from_slice(&body).expect("error response json");
        assert_eq!(parsed.code, "validation_error");
        let detail = parsed.detail.expect("validation detail should exist");
        assert!(detail
            .iter()
            .any(|issue| issue.loc.first() == Some(&"path".to_string())));
    }
}
