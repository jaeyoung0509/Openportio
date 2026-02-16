use std::sync::Arc;

use thiserror::Error;

pub mod auth;
pub mod error_mapping;

pub type OpenportioResult<T> = Result<T, OpenportioError>;
pub type MeldResult<T> = OpenportioResult<T>;
pub type AlloyResult<T> = OpenportioResult<T>;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub service_name: String,
    pub environment: String,
}

impl AppConfig {
    pub fn local(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            environment: "local".to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub enum OpenportioError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("internal error: {0}")]
    Internal(String),
}

macro_rules! impl_domain_error_mapping {
    ($error_ty:ty {
        $(
            $variant:ident => {
                rest_status: $rest_status:ident,
                rest_code: $rest_code:literal,
                grpc_code: $grpc_code:ident,
                expose_message: $expose_message:expr,
                issue_type: $issue_type:expr
            }
        ),+ $(,)?
    }) => {
        impl crate::error_mapping::DomainErrorDescriptor for $error_ty {
            fn domain_error_mapping(&self) -> crate::error_mapping::DomainErrorMapping {
                match self {
                    $(
                        Self::$variant(_) => crate::error_mapping::DomainErrorMapping {
                            rest_status: crate::error_mapping::DomainRestStatus::$rest_status,
                            rest_code: $rest_code,
                            grpc_code: crate::error_mapping::DomainGrpcCode::$grpc_code,
                            expose_message: $expose_message,
                            issue_type: $issue_type,
                        },
                    )+
                }
            }

            fn domain_error_message(&self) -> &str {
                match self {
                    $(
                        Self::$variant(message) => message.as_str(),
                    )+
                }
            }
        }
    };
}

impl_domain_error_mapping!(OpenportioError {
    Validation => {
        rest_status: BadRequest,
        rest_code: "validation_error",
        grpc_code: InvalidArgument,
        expose_message: true,
        issue_type: Some("domain_validation")
    },
    Internal => {
        rest_status: InternalServerError,
        rest_code: "internal_error",
        grpc_code: Internal,
        expose_message: false,
        issue_type: None
    }
});

pub type MeldError = OpenportioError;
pub type AlloyError = OpenportioError;

pub trait GreetingEngine: Send + Sync {
    fn greet(&self, name: &str) -> OpenportioResult<String>;
}

pub trait MetricsSink: Send + Sync {
    fn incr_counter(&self, name: &str);
}

#[derive(Debug, Default)]
pub struct NoopMetrics;

impl MetricsSink for NoopMetrics {
    fn incr_counter(&self, _name: &str) {}
}

#[derive(Debug, Clone)]
pub struct StaticGreetingEngine {
    prefix: String,
}

impl StaticGreetingEngine {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

impl GreetingEngine for StaticGreetingEngine {
    fn greet(&self, name: &str) -> OpenportioResult<String> {
        if name.trim().is_empty() {
            return Err(OpenportioError::Validation(
                "name must not be empty".to_string(),
            ));
        }
        Ok(format!("{}, {}!", self.prefix, name))
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub greeter: Arc<dyn GreetingEngine>,
    pub metrics: Arc<dyn MetricsSink>,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        greeter: Arc<dyn GreetingEngine>,
        metrics: Arc<dyn MetricsSink>,
    ) -> Self {
        Self {
            config,
            greeter,
            metrics,
        }
    }

    pub fn local(service_name: impl Into<String>) -> Self {
        Self {
            config: AppConfig::local(service_name),
            greeter: Arc::new(StaticGreetingEngine::new("Hello")),
            metrics: Arc::new(NoopMetrics),
        }
    }

    pub fn greet(&self, name: &str) -> OpenportioResult<String> {
        self.metrics.incr_counter("greet.requests");
        self.greeter.greet(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_mapping::{DomainErrorDescriptor, DomainGrpcCode, DomainRestStatus};

    #[test]
    fn greet_rejects_empty_name() {
        let state = AppState::local("openportio-test");
        let result = state.greet("  ");
        assert!(matches!(result, Err(OpenportioError::Validation(_))));
    }

    #[test]
    fn greet_returns_message() {
        let state = AppState::local("openportio-test");
        let result = state.greet("Rust");
        assert_eq!(result.expect("must greet"), "Hello, Rust!");
    }

    #[test]
    fn domain_error_mapping_is_declared_in_one_place() {
        let validation = OpenportioError::Validation("bad input".to_string());
        let validation_map = validation.domain_error_mapping();
        assert_eq!(validation_map.rest_status, DomainRestStatus::BadRequest);
        assert_eq!(validation_map.grpc_code, DomainGrpcCode::InvalidArgument);
        assert_eq!(validation_map.rest_code, "validation_error");
        assert!(validation_map.expose_message);
        assert_eq!(validation_map.issue_type, Some("domain_validation"));
        assert_eq!(validation.domain_error_message(), "bad input");

        let internal = OpenportioError::Internal("db exploded".to_string());
        let internal_map = internal.domain_error_mapping();
        assert_eq!(
            internal_map.rest_status,
            DomainRestStatus::InternalServerError
        );
        assert_eq!(internal_map.grpc_code, DomainGrpcCode::Internal);
        assert_eq!(internal_map.rest_code, "internal_error");
        assert!(!internal_map.expose_message);
        assert_eq!(internal_map.issue_type, None);
        assert_eq!(internal.domain_error_message(), "db exploded");
    }
}
