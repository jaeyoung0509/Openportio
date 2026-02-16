use std::sync::Arc;

use crate::auth::AuthRuntimeConfig;
use openportio_core::AppState;
use openportio_rpc::{
    build_hello_response, Greeter, GreeterServer, HelloRequest, HelloResponse, FILE_DESCRIPTOR_SET,
};
use tonic::service::Routes;
use tonic::{service::interceptor::InterceptedService, Request, Response, Status};

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

pub fn build_grpc_routes(state: Arc<AppState>) -> Routes {
    build_grpc_routes_with_auth(state, AuthRuntimeConfig::from_env())
}

pub fn build_grpc_routes_with_auth(state: Arc<AppState>, auth_cfg: AuthRuntimeConfig) -> Routes {
    build_grpc_routes_with_descriptor_set(state, auth_cfg, FILE_DESCRIPTOR_SET)
}

fn map_error(err: openportio_core::OpenportioError) -> Status {
    crate::api::map_domain_error_to_grpc(err)
}

fn build_grpc_routes_with_descriptor_set(
    state: Arc<AppState>,
    auth_cfg: AuthRuntimeConfig,
    descriptor_set: &'static [u8],
) -> Routes {
    let mut routes = Routes::new(build_grpc_service_with_auth(state, auth_cfg));

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
