use axum::{
    extract::{FromRequest, FromRequestParts, Path, Query},
    http::{request::Parts, StatusCode},
    response::IntoResponse,
    Json,
};
use openportio_core::{
    error_mapping::{DomainErrorDescriptor, DomainGrpcCode, DomainRestStatus},
    OpenportioError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tonic::Status;
use validator::{Validate, ValidationErrors};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
pub struct ApiValidationIssue {
    pub loc: Vec<String>,
    pub msg: String,
    #[serde(rename = "type")]
    pub issue_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApiErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ApiValidationIssue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiErrorResponse {
    pub fn validation(
        message: impl Into<String>,
        detail: Option<Vec<ApiValidationIssue>>,
        details: Option<Value>,
    ) -> Self {
        Self {
            code: "validation_error".to_string(),
            message: message.into(),
            detail,
            details,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            code: "bad_request".to_string(),
            message: message.into(),
            detail: None,
            details: None,
        }
    }

    pub fn internal_server_error() -> Self {
        Self {
            code: "internal_error".to_string(),
            message: "internal server error".to_string(),
            detail: None,
            details: None,
        }
    }
}

pub type ApiError = (StatusCode, Json<ApiErrorResponse>);

pub trait RequestValidation {
    fn validate_request(&self, source: &'static str) -> Result<(), ApiError>;
}

impl<T> RequestValidation for T
where
    T: Validate,
{
    fn validate_request(&self, source: &'static str) -> Result<(), ApiError> {
        self.validate()
            .map_err(|err| validation_error_with_source(err, source))
    }
}

pub fn validation_error(err: ValidationErrors) -> ApiError {
    validation_error_with_source(err, "request")
}

pub fn validation_error_with_source(err: ValidationErrors, source: &'static str) -> ApiError {
    let mut fields = serde_json::Map::new();
    let mut detail = Vec::new();
    for (field, errors) in err.field_errors() {
        let messages: Vec<String> = errors
            .iter()
            .map(|e| {
                e.message
                    .clone()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| e.code.to_string())
            })
            .collect();
        for error in errors {
            let msg = error
                .message
                .clone()
                .map(|m| m.to_string())
                .unwrap_or_else(|| error.code.to_string());
            detail.push(ApiValidationIssue {
                loc: vec![source.to_string(), field.to_string()],
                msg,
                issue_type: error.code.to_string(),
            });
        }
        fields.insert(field.to_string(), json!(messages));
    }

    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResponse::validation(
            "request validation failed",
            Some(detail),
            Some(Value::Object(fields)),
        )),
    )
}

pub fn bad_request(message: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResponse::bad_request(message)),
    )
}

pub fn map_domain_error_to_rest(err: OpenportioError) -> ApiError {
    let mapping = err.domain_error_mapping();
    let raw_message = err.domain_error_message().to_string();
    let rest_status = domain_rest_status_to_http(mapping.rest_status);

    if rest_status == StatusCode::INTERNAL_SERVER_ERROR && !mapping.expose_message {
        tracing::error!(error = %raw_message, "internal domain error surfaced in REST handler");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResponse::internal_server_error()),
        );
    }

    let mut detail = None;
    if let Some(issue_type) = mapping.issue_type {
        detail = Some(vec![ApiValidationIssue {
            loc: vec!["domain".to_string()],
            msg: raw_message.clone(),
            issue_type: issue_type.to_string(),
        }]);
    }

    if rest_status == StatusCode::BAD_REQUEST && mapping.rest_code == "validation_error" {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse::validation(raw_message, detail, None)),
        );
    }

    let response_message = if mapping.expose_message {
        raw_message
    } else {
        default_rest_client_message(rest_status).to_string()
    };

    (
        rest_status,
        Json(ApiErrorResponse {
            code: mapping.rest_code.to_string(),
            message: response_message,
            detail,
            details: None,
        }),
    )
}

fn domain_rest_status_to_http(status: DomainRestStatus) -> StatusCode {
    match status {
        DomainRestStatus::BadRequest => StatusCode::BAD_REQUEST,
        DomainRestStatus::Unauthorized => StatusCode::UNAUTHORIZED,
        DomainRestStatus::Forbidden => StatusCode::FORBIDDEN,
        DomainRestStatus::NotFound => StatusCode::NOT_FOUND,
        DomainRestStatus::Conflict => StatusCode::CONFLICT,
        DomainRestStatus::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
        DomainRestStatus::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn default_rest_client_message(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::TOO_MANY_REQUESTS => "too many requests",
        _ => "internal server error",
    }
}

pub fn map_domain_error_to_grpc(err: OpenportioError) -> Status {
    let mapping = err.domain_error_mapping();
    let raw_message = err.domain_error_message().to_string();

    if matches!(mapping.grpc_code, DomainGrpcCode::Internal) && !mapping.expose_message {
        tracing::error!(error = %raw_message, "internal domain error surfaced in gRPC handler");
    }

    let grpc_message = if mapping.expose_message {
        raw_message
    } else {
        "internal server error".to_string()
    };

    Status::new(domain_grpc_code_to_tonic(mapping.grpc_code), grpc_message)
}

fn domain_grpc_code_to_tonic(code: DomainGrpcCode) -> tonic::Code {
    match code {
        DomainGrpcCode::InvalidArgument => tonic::Code::InvalidArgument,
        DomainGrpcCode::Unauthenticated => tonic::Code::Unauthenticated,
        DomainGrpcCode::PermissionDenied => tonic::Code::PermissionDenied,
        DomainGrpcCode::NotFound => tonic::Code::NotFound,
        DomainGrpcCode::AlreadyExists => tonic::Code::AlreadyExists,
        DomainGrpcCode::ResourceExhausted => tonic::Code::ResourceExhausted,
        DomainGrpcCode::Internal => tonic::Code::Internal,
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedJson<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + RequestValidation,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|err| bad_request(format!("invalid json body: {err}")))?;

        value.validate_request("body")?;
        Ok(Self(value))
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedQuery<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned + RequestValidation,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|err| bad_request(format!("invalid query: {err}")))?;
        value.validate_request("query")?;
        Ok(Self(value))
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedPath<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequestParts<S> for ValidatedPath<T>
where
    T: DeserializeOwned + RequestValidation + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(value) = Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(|err| bad_request(format!("invalid path: {err}")))?;
        value.validate_request("path")?;
        Ok(Self(value))
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedParts<T>(pub T);

#[axum::async_trait]
impl<T, S> FromRequestParts<S> for ValidatedParts<T>
where
    T: FromRequestParts<S> + RequestValidation,
    T::Rejection: std::fmt::Display,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let value = T::from_request_parts(parts, state)
            .await
            .map_err(|err| bad_request(format!("invalid request parts: {err}")))?;
        value.validate_request("parts")?;
        Ok(Self(value))
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> axum::response::Response {
        Json(self).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
        routing::post,
        Router,
    };
    use tower::util::ServiceExt;
    use validator::Validate;

    #[derive(Debug, serde::Deserialize, Validate)]
    struct BodyDto {
        #[validate(length(min = 3))]
        name: String,
    }

    #[test]
    fn validation_error_includes_fastapi_like_detail_shape() {
        let dto = BodyDto {
            name: "ab".to_string(),
        };
        let err = dto.validate().expect_err("must fail");
        let (status, Json(body)) = validation_error_with_source(err, "body");

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.code, "validation_error");
        let detail = body.detail.expect("detail should exist");
        assert!(detail
            .iter()
            .any(|issue| issue.loc == vec!["body".to_string(), "name".to_string()]));
    }

    #[test]
    fn internal_domain_errors_are_sanitized_for_rest_clients() {
        let (status, Json(body)) =
            map_domain_error_to_rest(OpenportioError::Internal("db exploded".to_string()));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.code, "internal_error");
        assert_eq!(body.message, "internal server error");
        assert!(body.detail.is_none());
    }

    #[test]
    fn internal_domain_errors_are_sanitized_for_grpc_clients() {
        let status = map_domain_error_to_grpc(OpenportioError::Internal("db exploded".to_string()));
        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), "internal server error");
    }

    #[derive(Debug, serde::Deserialize, Validate)]
    struct DerivedValidationDto {
        #[validate(length(min = 3))]
        name: String,
    }

    #[derive(Debug, serde::Deserialize)]
    struct TraitFirstDto {
        name: String,
    }

    impl RequestValidation for TraitFirstDto {
        fn validate_request(&self, source: &'static str) -> Result<(), ApiError> {
            if self.name.starts_with("admin") {
                let issue = ApiValidationIssue {
                    loc: vec![source.to_string(), "name".to_string()],
                    msg: "admin-prefixed names are reserved".to_string(),
                    issue_type: "custom_validation".to_string(),
                };
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse::validation(
                        "request validation failed",
                        Some(vec![issue]),
                        None,
                    )),
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn request_validation_trait_matches_validator_flow_for_common_case() {
        let derived = DerivedValidationDto {
            name: "ab".to_string(),
        };
        let (derived_status, Json(derived_body)) = derived
            .validate_request("body")
            .expect_err("validator-backed dto must fail");
        assert_eq!(derived_status, StatusCode::BAD_REQUEST);
        assert_eq!(derived_body.code, "validation_error");

        let trait_first = TraitFirstDto {
            name: "admin-test".to_string(),
        };
        let (trait_status, Json(trait_body)) = trait_first
            .validate_request("body")
            .expect_err("trait-first dto must fail");
        assert_eq!(trait_status, StatusCode::BAD_REQUEST);
        assert_eq!(trait_body.code, "validation_error");
        let detail = trait_body.detail.expect("detail should exist");
        assert!(detail
            .iter()
            .any(|issue| issue.issue_type == "custom_validation"));
    }

    async fn trait_first_handler(
        ValidatedJson(body): ValidatedJson<TraitFirstDto>,
    ) -> Result<Json<String>, ApiError> {
        Ok(Json(body.name))
    }

    #[tokio::test]
    async fn validated_json_accepts_trait_first_request_validation() {
        let app = Router::new().route("/trait-first", post(trait_first_handler));

        let ok_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/trait-first")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"rustacean"}"#))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(ok_response.status(), StatusCode::OK);

        let bad_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/trait-first")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"admin-root"}"#))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(bad_response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(bad_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ApiErrorResponse = serde_json::from_slice(&body).expect("api error json");
        assert_eq!(parsed.code, "validation_error");
    }
}
