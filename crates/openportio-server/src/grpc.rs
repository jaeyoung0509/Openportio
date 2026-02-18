use std::{convert::Infallible, future::Future, sync::Arc};

use crate::auth::AuthRuntimeConfig;
use axum::extract::FromRef;
use http::{Request as HttpRequest, Response as HttpResponse};
use openportio_core::{auth::AuthPrincipal, AppState, OpenportioError};
use openportio_rpc::{
    build_hello_response, Greeter, GreeterServer, HelloRequest, HelloResponse, FILE_DESCRIPTOR_SET,
};
use tonic::service::Routes;
use tonic::{body::BoxBody, server::NamedService};
use tonic::{service::interceptor::InterceptedService, Request, Response, Status};
use tower::Service;
use validator::Validate;

pub type GrpcHelloRequest = HelloRequest;
pub type GrpcHelloResponse = HelloResponse;

#[derive(Clone)]
pub struct GrpcHandlerContext {
    state: Arc<AppState>,
    principal: AuthPrincipal,
}

impl GrpcHandlerContext {
    pub fn state(&self) -> Arc<AppState> {
        self.state.clone()
    }

    pub fn principal(&self) -> &AuthPrincipal {
        &self.principal
    }

    pub fn depends<T>(&self) -> T
    where
        T: FromRef<Arc<AppState>>,
    {
        T::from_ref(&self.state)
    }
}

pub trait GrpcRequestValidation {
    fn validate_grpc_request(&self) -> Result<(), OpenportioError>;
}

impl<T> GrpcRequestValidation for T
where
    T: Validate,
{
    fn validate_grpc_request(&self) -> Result<(), OpenportioError> {
        self.validate()
            .map_err(|err| OpenportioError::Validation(format!("request validation failed: {err}")))
    }
}

pub fn validated_grpc_request<T>(value: T) -> Result<T, OpenportioError>
where
    T: GrpcRequestValidation,
{
    value.validate_grpc_request()?;
    Ok(value)
}

#[derive(Clone)]
pub struct GreeterService {
    state: Arc<AppState>,
}

impl GreeterService {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl Greeter for GreeterService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloResponse>, Status> {
        let response =
            build_hello_response(&self.state, request.into_inner()).map_err(map_error)?;
        Ok(Response::new(response))
    }
}

#[derive(Clone)]
pub struct GreeterFnService<H> {
    state: Arc<AppState>,
    say_hello: Arc<H>,
}

impl<H> GreeterFnService<H> {
    pub fn new(state: Arc<AppState>, say_hello: H) -> Self {
        Self {
            state,
            say_hello: Arc::new(say_hello),
        }
    }
}

#[derive(Clone)]
pub struct GreeterContextFnService<H> {
    state: Arc<AppState>,
    say_hello: Arc<H>,
}

impl<H> GreeterContextFnService<H> {
    pub fn new(state: Arc<AppState>, say_hello: H) -> Self {
        Self {
            state,
            say_hello: Arc::new(say_hello),
        }
    }
}

#[tonic::async_trait]
impl<H, Fut> Greeter for GreeterFnService<H>
where
    H: Fn(Arc<AppState>, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    async fn say_hello(
        &self,
        request: Request<GrpcHelloRequest>,
    ) -> Result<Response<GrpcHelloResponse>, Status> {
        let response = (self.say_hello.as_ref())(self.state.clone(), request.into_inner())
            .await
            .map_err(map_error)?;
        Ok(Response::new(response))
    }
}

#[tonic::async_trait]
impl<H, Fut> Greeter for GreeterContextFnService<H>
where
    H: Fn(GrpcHandlerContext, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    async fn say_hello(
        &self,
        request: Request<GrpcHelloRequest>,
    ) -> Result<Response<GrpcHelloResponse>, Status> {
        let principal = request
            .extensions()
            .get::<AuthPrincipal>()
            .cloned()
            .unwrap_or_else(anonymous_principal);
        let context = GrpcHandlerContext {
            state: self.state.clone(),
            principal,
        };

        let response = (self.say_hello.as_ref())(context, request.into_inner())
            .await
            .map_err(map_error)?;
        Ok(Response::new(response))
    }
}

pub fn build_grpc_service(
    state: Arc<AppState>,
) -> InterceptedService<GreeterServer<GreeterService>, GrpcAuthInterceptor> {
    build_grpc_service_with_auth(state, AuthRuntimeConfig::from_env())
}

pub fn build_grpc_service_with_auth(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
) -> InterceptedService<GreeterServer<GreeterService>, GrpcAuthInterceptor> {
    let service = GreeterServer::new(GreeterService::new(state));
    InterceptedService::new(service, GrpcAuthInterceptor { auth_cfg })
}

pub fn build_grpc_service_from_say_hello_handler<H, Fut>(
    state: Arc<AppState>,
    say_hello: H,
) -> InterceptedService<GreeterServer<GreeterFnService<H>>, GrpcAuthInterceptor>
where
    H: Fn(Arc<AppState>, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    build_grpc_service_from_say_hello_handler_with_auth(
        state,
        AuthRuntimeConfig::from_env(),
        say_hello,
    )
}

pub fn build_grpc_service_from_say_hello_handler_with_auth<H, Fut>(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
    say_hello: H,
) -> InterceptedService<GreeterServer<GreeterFnService<H>>, GrpcAuthInterceptor>
where
    H: Fn(Arc<AppState>, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    let service = GreeterServer::new(GreeterFnService::new(state, say_hello));
    InterceptedService::new(service, GrpcAuthInterceptor { auth_cfg })
}

pub fn build_grpc_service_from_say_hello_context_handler<H, Fut>(
    state: Arc<AppState>,
    say_hello: H,
) -> InterceptedService<GreeterServer<GreeterContextFnService<H>>, GrpcAuthInterceptor>
where
    H: Fn(GrpcHandlerContext, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    build_grpc_service_from_say_hello_context_handler_with_auth(
        state,
        AuthRuntimeConfig::from_env(),
        say_hello,
    )
}

pub fn build_grpc_service_from_say_hello_context_handler_with_auth<H, Fut>(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
    say_hello: H,
) -> InterceptedService<GreeterServer<GreeterContextFnService<H>>, GrpcAuthInterceptor>
where
    H: Fn(GrpcHandlerContext, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    let service = GreeterServer::new(GreeterContextFnService::new(state, say_hello));
    InterceptedService::new(service, GrpcAuthInterceptor { auth_cfg })
}

pub fn build_grpc_routes(state: Arc<AppState>) -> Routes {
    build_grpc_routes_with_auth(state, AuthRuntimeConfig::from_env())
}

pub fn build_grpc_routes_with_auth(state: Arc<AppState>, auth_cfg: AuthRuntimeConfig) -> Routes {
    build_grpc_routes_with_descriptor_set(state, auth_cfg, FILE_DESCRIPTOR_SET)
}

pub fn build_grpc_routes_from_say_hello_handler<H, Fut>(
    state: Arc<AppState>,
    say_hello: H,
) -> Routes
where
    H: Fn(Arc<AppState>, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    build_grpc_routes_from_say_hello_handler_with_auth(
        state,
        AuthRuntimeConfig::from_env(),
        say_hello,
    )
}

pub fn build_grpc_routes_from_say_hello_handler_with_auth<H, Fut>(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
    say_hello: H,
) -> Routes
where
    H: Fn(Arc<AppState>, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    build_grpc_routes_with_descriptor_service(
        build_grpc_service_from_say_hello_handler_with_auth(state, auth_cfg, say_hello),
        FILE_DESCRIPTOR_SET,
    )
}

pub fn build_grpc_routes_from_say_hello_context_handler<H, Fut>(
    state: Arc<AppState>,
    say_hello: H,
) -> Routes
where
    H: Fn(GrpcHandlerContext, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    build_grpc_routes_from_say_hello_context_handler_with_auth(
        state,
        AuthRuntimeConfig::from_env(),
        say_hello,
    )
}

pub fn build_grpc_routes_from_say_hello_context_handler_with_auth<H, Fut>(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
    say_hello: H,
) -> Routes
where
    H: Fn(GrpcHandlerContext, GrpcHelloRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<GrpcHelloResponse, OpenportioError>> + Send + 'static,
{
    build_grpc_routes_with_descriptor_service(
        build_grpc_service_from_say_hello_context_handler_with_auth(state, auth_cfg, say_hello),
        FILE_DESCRIPTOR_SET,
    )
}

fn map_error(err: openportio_core::OpenportioError) -> Status {
    crate::api::map_domain_error_to_grpc(err)
}

fn anonymous_principal() -> AuthPrincipal {
    AuthPrincipal {
        subject: "anonymous".to_string(),
        issuer: None,
        audience: vec![],
        scopes: vec![],
    }
}

fn build_grpc_routes_with_descriptor_set(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
    descriptor_set: &'static [u8],
) -> Routes {
    build_grpc_routes_with_descriptor_service(
        build_grpc_service_with_auth(state, auth_cfg),
        descriptor_set,
    )
}

fn build_grpc_routes_with_descriptor_service<S>(service: S, descriptor_set: &'static [u8]) -> Routes
where
    S: Service<HttpRequest<BoxBody>, Response = HttpResponse<BoxBody>, Error = Infallible>
        + NamedService
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let mut routes = Routes::new(service);

    match tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(descriptor_set)
        .build_v1()
    {
        Ok(service) => {
            routes = routes.add_service(service);
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "failed to initialize grpc reflection v1; continuing without v1 reflection"
            );
        }
    }

    match tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(descriptor_set)
        .build_v1alpha()
    {
        Ok(service) => {
            routes = routes.add_service(service);
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "failed to initialize grpc reflection v1alpha; continuing without v1alpha reflection"
            );
        }
    }

    routes.prepare()
}

#[derive(Clone)]
pub struct GrpcAuthInterceptor {
    auth_cfg: AuthRuntimeConfig,
}

impl tonic::service::Interceptor for GrpcAuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if !self.auth_cfg.enabled {
            request.extensions_mut().insert(anonymous_principal());
            return Ok(request);
        }

        let auth_value = request
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("missing bearer token"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("authorization metadata is invalid"))?;

        let principal = self
            .auth_cfg
            .authenticate_authorization_value_str(auth_value)
            .map_err(|err| err.into_grpc_status())?;
        request.extensions_mut().insert(principal);
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::FromRef;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use tonic::service::Interceptor;

    #[derive(Debug, Clone)]
    struct ServiceLabel(String);

    impl FromRef<Arc<AppState>> for ServiceLabel {
        fn from_ref(state: &Arc<AppState>) -> Self {
            Self(state.config.service_name.clone())
        }
    }

    #[derive(Debug, validator::Validate)]
    struct HelloInput {
        #[validate(length(min = 1))]
        name: String,
    }

    #[tokio::test]
    async fn greeter_fn_service_supports_fastapi_like_happy_path() {
        let service = GreeterFnService::new(
            Arc::new(AppState::local("grpc-dx-happy")),
            |_state, request: GrpcHelloRequest| async move {
                Ok(GrpcHelloResponse {
                    message: format!("hello from handler, {}", request.name),
                })
            },
        );

        let response = Greeter::say_hello(
            &service,
            Request::new(GrpcHelloRequest {
                name: "Rust".to_string(),
            }),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(
            response.into_inner().message,
            "hello from handler, Rust".to_string()
        );
    }

    #[tokio::test]
    async fn greeter_fn_service_maps_domain_failure_to_grpc_status() {
        let service = GreeterFnService::new(
            Arc::new(AppState::local("grpc-dx-failure")),
            |_state, _request: GrpcHelloRequest| async move {
                Err(OpenportioError::Validation("name is invalid".to_string()))
            },
        );

        let status = Greeter::say_hello(
            &service,
            Request::new(GrpcHelloRequest {
                name: "".to_string(),
            }),
        )
        .await
        .expect_err("handler should fail");

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert_eq!(status.message(), "name is invalid");
    }

    #[tokio::test]
    async fn greeter_context_service_supports_validation_di_and_principal() {
        let service = GreeterContextFnService::new(
            Arc::new(AppState::local("grpc-dx-context")),
            |ctx: GrpcHandlerContext, request: GrpcHelloRequest| async move {
                let input = validated_grpc_request(HelloInput { name: request.name })?;
                let label = ctx.depends::<ServiceLabel>();
                Ok(GrpcHelloResponse {
                    message: format!(
                        "[{}] {} says hello to {}",
                        label.0,
                        ctx.principal().subject,
                        input.name
                    ),
                })
            },
        );

        let mut request = Request::new(GrpcHelloRequest {
            name: "Rust".to_string(),
        });
        request.extensions_mut().insert(AuthPrincipal {
            subject: "user-1".to_string(),
            issuer: Some("https://issuer.local".to_string()),
            audience: vec!["openportio-api".to_string()],
            scopes: vec!["read:notes".to_string()],
        });

        let response = Greeter::say_hello(&service, request)
            .await
            .expect("handler should succeed");
        assert_eq!(
            response.into_inner().message,
            "[grpc-dx-context] user-1 says hello to Rust"
        );
    }

    #[tokio::test]
    async fn greeter_context_service_maps_validation_failure_to_grpc_status() {
        let service = GreeterContextFnService::new(
            Arc::new(AppState::local("grpc-dx-context-validation")),
            |_ctx: GrpcHandlerContext, request: GrpcHelloRequest| async move {
                let _ = validated_grpc_request(HelloInput { name: request.name })?;
                Ok(GrpcHelloResponse {
                    message: "ok".to_string(),
                })
            },
        );

        let status = Greeter::say_hello(
            &service,
            Request::new(GrpcHelloRequest {
                name: "".to_string(),
            }),
        )
        .await
        .expect_err("handler should fail");

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(status.message().contains("request validation failed"));
    }

    #[tokio::test]
    async fn greeter_context_service_sanitizes_internal_failures() {
        let service = GreeterContextFnService::new(
            Arc::new(AppState::local("grpc-dx-context-internal")),
            |_ctx: GrpcHandlerContext, _request: GrpcHelloRequest| async move {
                Err(OpenportioError::Internal("db exploded".to_string()))
            },
        );

        let status = Greeter::say_hello(
            &service,
            Request::new(GrpcHelloRequest {
                name: "Rust".to_string(),
            }),
        )
        .await
        .expect_err("handler should fail");

        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), "internal server error");
    }

    #[derive(serde::Serialize)]
    struct TestClaims {
        sub: String,
        exp: usize,
        iss: String,
        aud: String,
    }

    fn issue_test_token(secret: &str) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            &TestClaims {
                sub: "user-1".to_string(),
                exp: 4_102_444_800,
                iss: "https://issuer.local".to_string(),
                aud: "openportio-api".to_string(),
            },
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token should encode")
    }

    #[test]
    fn grpc_auth_interceptor_rejects_missing_token_when_auth_enabled() {
        let mut auth_cfg = AuthRuntimeConfig::default();
        auth_cfg.enabled = true;
        auth_cfg.jwt_secret = Some("dev-secret".to_string());
        auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
        auth_cfg.expected_audience = Some("openportio-api".to_string());
        let mut interceptor = GrpcAuthInterceptor { auth_cfg };

        let status = interceptor
            .call(Request::new(()))
            .expect_err("missing token should fail");
        assert_eq!(status.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn grpc_auth_interceptor_injects_principal_for_valid_token() {
        let mut auth_cfg = AuthRuntimeConfig::default();
        auth_cfg.enabled = true;
        auth_cfg.jwt_secret = Some("dev-secret".to_string());
        auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
        auth_cfg.expected_audience = Some("openportio-api".to_string());
        let mut interceptor = GrpcAuthInterceptor { auth_cfg };

        let token = issue_test_token("dev-secret");
        let mut request = Request::new(());
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().expect("header value"),
        );

        let request = interceptor.call(request).expect("valid token should pass");
        let principal = request
            .extensions()
            .get::<AuthPrincipal>()
            .expect("principal should be injected");
        assert_eq!(principal.subject, "user-1");
    }

    #[test]
    fn grpc_auth_interceptor_injects_anonymous_principal_when_auth_disabled() {
        let mut interceptor = GrpcAuthInterceptor {
            auth_cfg: AuthRuntimeConfig::default(),
        };
        let request = interceptor
            .call(Request::new(()))
            .expect("auth disabled should pass");
        let principal = request
            .extensions()
            .get::<AuthPrincipal>()
            .expect("principal should be injected");
        assert_eq!(principal.subject, "anonymous");
    }

    #[test]
    fn invalid_descriptor_set_does_not_panic_grpc_route_build() {
        const INVALID_DESCRIPTOR_SET: &[u8] = b"invalid-descriptor-set";

        let result = std::panic::catch_unwind(|| {
            let _ = build_grpc_routes_with_descriptor_set(
                Arc::new(AppState::local("grpc-reflection-invalid-descriptor")),
                AuthRuntimeConfig::default(),
                INVALID_DESCRIPTOR_SET,
            );
        });

        assert!(result.is_ok());
    }
}
