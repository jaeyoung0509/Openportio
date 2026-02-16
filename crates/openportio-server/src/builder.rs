use std::{convert::Infallible, env, future::IntoFuture, io, net::SocketAddr, sync::Arc};

use axum::Router;
use http::{Request, Response};
use openportio_core::AppState;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tonic::{body::BoxBody, server::NamedService, service::Routes};
use tower::Service;

use crate::{build_router, di, grpc, middleware};

type RouterCustomizer = Box<dyn Fn(Router) -> Router + Send + Sync + 'static>;
type StartupHook = Box<dyn Fn(SocketAddr) + Send + Sync + 'static>;
type ShutdownHook = Box<dyn Fn() + Send + Sync + 'static>;

pub struct OpenportioServer {
    state: Arc<AppState>,
    addr: SocketAddr,
    rest_addr: Option<SocketAddr>,
    grpc_addr: Option<SocketAddr>,
    rest_router: Option<Router>,
    raw_routers: Vec<Router>,
    grpc_routes: Option<Routes>,
    dependency_overrides: di::DependencyOverrides,
    middleware_config: middleware::MiddlewareConfig,
    middleware_customizers: Vec<RouterCustomizer>,
    startup_hooks: Vec<StartupHook>,
    shutdown_hooks: Vec<ShutdownHook>,
}

impl OpenportioServer {
    pub fn new() -> Self {
        let state = Arc::new(AppState::local("openportio-server"));
        Self {
            grpc_routes: Some(grpc::build_grpc_routes(state.clone())),
            state,
            addr: load_addr_from_env().unwrap_or(SocketAddr::from(([127, 0, 0, 1], 3000))),
            rest_addr: None,
            grpc_addr: None,
            rest_router: None,
            raw_routers: Vec::new(),
            dependency_overrides: di::DependencyOverrides::default(),
            middleware_config: middleware::MiddlewareConfig::from_env(),
            middleware_customizers: Vec::new(),
            startup_hooks: Vec::new(),
            shutdown_hooks: Vec::new(),
        }
    }

    pub fn with_addr(mut self, addr: SocketAddr) -> Self {
        self.addr = addr;
        self.rest_addr = None;
        self.grpc_addr = None;
        self
    }

    pub fn with_rest_addr(mut self, addr: SocketAddr) -> Self {
        self.rest_addr = Some(addr);
        self
    }

    pub fn with_grpc_addr(mut self, addr: SocketAddr) -> Self {
        self.grpc_addr = Some(addr);
        self
    }

    pub fn with_state(mut self, state: Arc<AppState>) -> Self {
        self.state = state;
        self
    }

    pub fn with_rest_router(mut self, router: Router) -> Self {
        self.rest_router = Some(router);
        self
    }

    pub fn merge_raw_router(mut self, router: Router) -> Self {
        self.raw_routers.push(router);
        self
    }

    pub fn with_dependency<T>(mut self, value: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.dependency_overrides = self.dependency_overrides.with(value);
        self
    }

    pub fn without_grpc(mut self) -> Self {
        self.grpc_routes = None;
        self
    }

    pub fn with_grpc_service<S>(mut self, service: S) -> Self
    where
        S: Service<Request<BoxBody>, Response = Response<BoxBody>, Error = Infallible>
            + NamedService
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        let routes = match self.grpc_routes.take() {
            Some(existing) => existing.add_service(service).prepare(),
            None => Routes::new(service).prepare(),
        };
        self.grpc_routes = Some(routes);
        self
    }

    pub fn with_grpc_routes(mut self, routes: Routes) -> Self {
        self.grpc_routes = Some(routes.prepare());
        self
    }

    pub fn configure_tonic<F>(self, configure: F) -> Self
    where
        F: FnOnce(Routes) -> Routes,
    {
        self.configure_tonic_routes(configure)
    }

    pub fn configure_tonic_routes<F>(mut self, configure: F) -> Self
    where
        F: FnOnce(Routes) -> Routes,
    {
        self.grpc_routes = self
            .grpc_routes
            .take()
            .map(|routes| configure(routes).prepare());
        self
    }

    pub fn with_middleware_config(mut self, config: middleware::MiddlewareConfig) -> Self {
        self.middleware_config = config;
        self
    }

    pub fn with_middleware<F>(mut self, f: F) -> Self
    where
        F: Fn(Router) -> Router + Send + Sync + 'static,
    {
        self.middleware_customizers.push(Box::new(f));
        self
    }

    pub fn on_startup<F>(mut self, hook: F) -> Self
    where
        F: Fn(SocketAddr) + Send + Sync + 'static,
    {
        self.startup_hooks.push(Box::new(hook));
        self
    }

    pub fn on_shutdown<F>(mut self, hook: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.shutdown_hooks.push(Box::new(hook));
        self
    }

    pub fn build_app(&self) -> Router {
        let merged = match self.build_grpc_router() {
            Some(grpc_router) => self.build_rest_router().merge(grpc_router),
            None => self.build_rest_router(),
        };
        self.finalize_router(merged)
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self.dual_port_addrs()? {
            Some((rest_addr, grpc_addr)) => self.run_dual_port(rest_addr, grpc_addr).await,
            None => self.run_single_port().await,
        }
    }

    fn build_rest_router(&self) -> Router {
        let rest = self
            .rest_router
            .clone()
            .unwrap_or_else(|| build_router(self.state.clone()));
        self.raw_routers
            .iter()
            .cloned()
            .fold(rest, |acc, router| acc.merge(router))
    }

    fn build_grpc_router(&self) -> Option<Router> {
        self.grpc_routes
            .clone()
            .map(|routes| routes.into_axum_router())
    }

    fn finalize_router(&self, router: Router) -> Router {
        let app = middleware::apply_shared_middleware(router, &self.middleware_config);
        let app = di::with_dependency_overrides(app, self.dependency_overrides.clone());
        self.middleware_customizers
            .iter()
            .fold(app, |acc, customizer| customizer(acc))
    }

    fn dual_port_addrs(
        &self,
    ) -> Result<Option<(SocketAddr, SocketAddr)>, Box<dyn std::error::Error>> {
        match (self.rest_addr, self.grpc_addr) {
            (None, None) => Ok(None),
            (Some(rest), Some(grpc)) => Ok(Some((rest, grpc))),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "dual-port mode requires both with_rest_addr(...) and with_grpc_addr(...)",
            )
            .into()),
        }
    }

    async fn run_single_port(self) -> Result<(), Box<dyn std::error::Error>> {
        let app = self.build_app();
        let listener = TcpListener::bind(self.addr).await?;

        for hook in &self.startup_hooks {
            hook(self.addr);
        }
        tracing::info!(addr = %self.addr, "openportio-server listening");

        let shutdown_hooks = self.shutdown_hooks;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = tokio::signal::ctrl_c().await;
                for hook in &shutdown_hooks {
                    hook();
                }
            })
            .await?;
        Ok(())
    }

    async fn run_dual_port(
        self,
        rest_addr: SocketAddr,
        grpc_addr: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let grpc_router = self.build_grpc_router().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "dual-port mode requires gRPC routes; do not combine with without_grpc()",
            )
        })?;
        let rest_app = self.finalize_router(self.build_rest_router());
        let grpc_app = self.finalize_router(grpc_router);

        let rest_listener = TcpListener::bind(rest_addr).await?;
        let grpc_listener = TcpListener::bind(grpc_addr).await?;

        for hook in &self.startup_hooks {
            hook(rest_addr);
        }
        for hook in &self.startup_hooks {
            hook(grpc_addr);
        }
        tracing::info!(addr = %rest_addr, mode = "dual-port", protocol = "rest", "openportio-server listening");
        tracing::info!(addr = %grpc_addr, mode = "dual-port", protocol = "grpc", "openportio-server listening");

        let (shutdown_tx, _) = watch::channel(false);
        let mut rest_shutdown = shutdown_tx.subscribe();
        let mut grpc_shutdown = shutdown_tx.subscribe();

        let rest_server = axum::serve(rest_listener, rest_app)
            .with_graceful_shutdown(async move {
                let _ = rest_shutdown.changed().await;
            })
            .into_future();
        let grpc_server = axum::serve(grpc_listener, grpc_app)
            .with_graceful_shutdown(async move {
                let _ = grpc_shutdown.changed().await;
            })
            .into_future();

        tokio::pin!(rest_server);
        tokio::pin!(grpc_server);

        let first_exit = tokio::select! {
            _ = tokio::signal::ctrl_c() => None,
            result = &mut rest_server => Some(("rest", result)),
            result = &mut grpc_server => Some(("grpc", result)),
        };

        let _ = shutdown_tx.send(true);

        let (rest_result, grpc_result) = match first_exit {
            Some(("rest", result)) => (result, grpc_server.await),
            Some(("grpc", result)) => (rest_server.await, result),
            None => (rest_server.await, grpc_server.await),
            Some((_other, _result)) => unreachable!("only rest/grpc branches are possible"),
        };

        for hook in &self.shutdown_hooks {
            hook();
        }

        rest_result?;
        grpc_result?;
        Ok(())
    }
}

impl Default for OpenportioServer {
    fn default() -> Self {
        Self::new()
    }
}

fn load_addr_from_env() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    match read_env_with_aliases(&[
        "OPENPORTIO_SERVER_ADDR",
        "MELD_SERVER_ADDR",
        "ALLOY_SERVER_ADDR",
    ]) {
        Ok(raw) => Ok(raw.parse()?),
        Err(env::VarError::NotPresent) => Ok(SocketAddr::from(([127, 0, 0, 1], 3000))),
        Err(err) => Err(Box::new(err)),
    }
}

fn read_env_with_aliases(names: &[&str]) -> Result<String, env::VarError> {
    for name in names {
        match env::var(name) {
            Ok(value) => return Ok(value),
            Err(env::VarError::NotPresent) => continue,
            Err(err) => return Err(err),
        }
    }
    Err(env::VarError::NotPresent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::to_bytes,
        body::Body,
        extract::FromRef,
        http::{Request, StatusCode},
        routing::get,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    };
    use tonic::service::Routes;
    use tower::util::ServiceExt;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[tokio::test]
    async fn builder_creates_working_app() {
        let app = OpenportioServer::new().build_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("health request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn builder_supports_custom_rest_and_middleware_chain() {
        let custom_router = Router::new().route("/custom", get(|| async { "custom-ok" }));
        let app = OpenportioServer::new()
            .without_grpc()
            .with_rest_router(custom_router)
            .with_middleware(|router| router.route("/ping", get(|| async { "pong" })))
            .build_app();

        let ping_response = app
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .expect("ping request should succeed");
        assert_eq!(ping_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn builder_supports_raw_router_merge() {
        let raw_router = Router::new().route("/raw-metrics", get(|| async { "metrics-ok" }));
        let app = OpenportioServer::new()
            .without_grpc()
            .merge_raw_router(raw_router)
            .build_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/raw-metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("metrics request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(
            String::from_utf8(body.to_vec()).expect("utf8"),
            "metrics-ok"
        );
    }

    #[tokio::test]
    async fn builder_supports_tonic_routes_configuration_hook() {
        let app = OpenportioServer::new()
            .configure_tonic(|routes| {
                let router = routes
                    .into_axum_router()
                    .route("/grpc-hook", get(|| async { "grpc-hook-ok" }));
                Routes::from(router)
            })
            .build_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/grpc-hook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("grpc hook request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(
            String::from_utf8(body.to_vec()).expect("utf8"),
            "grpc-hook-ok"
        );
    }

    #[test]
    fn configure_tonic_is_noop_when_grpc_is_disabled() {
        let called = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&called);
        let _ = OpenportioServer::new()
            .without_grpc()
            .configure_tonic(move |routes| {
                marker.store(true, Ordering::SeqCst);
                routes
            })
            .build_app();
        assert!(!called.load(Ordering::SeqCst));
    }

    #[derive(Clone)]
    struct LabelDep(String);

    impl FromRef<Arc<AppState>> for LabelDep {
        fn from_ref(_state: &Arc<AppState>) -> Self {
            Self("from-state".to_string())
        }
    }

    async fn dep_handler(crate::di::Depends(dep): crate::di::Depends<LabelDep>) -> String {
        dep.0
    }

    #[tokio::test]
    async fn builder_supports_dependency_overrides() {
        let app = OpenportioServer::new()
            .without_grpc()
            .with_rest_router(
                Router::new()
                    .route("/dep", get(dep_handler))
                    .with_state(Arc::new(AppState::local("builder-test"))),
            )
            .with_dependency(LabelDep("override".to_string()))
            .build_app();

        let response = app
            .oneshot(Request::builder().uri("/dep").body(Body::empty()).unwrap())
            .await
            .expect("dep request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(String::from_utf8(body.to_vec()).expect("utf8"), "override");
    }

    #[test]
    fn load_addr_supports_meld_compatibility_alias() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        for key in [
            "OPENPORTIO_SERVER_ADDR",
            "MELD_SERVER_ADDR",
            "ALLOY_SERVER_ADDR",
        ] {
            env::remove_var(key);
        }
        env::set_var("MELD_SERVER_ADDR", "127.0.0.1:4310");
        let addr = load_addr_from_env().expect("addr should parse");
        assert_eq!(addr, SocketAddr::from(([127, 0, 0, 1], 4310)));
        for key in [
            "OPENPORTIO_SERVER_ADDR",
            "MELD_SERVER_ADDR",
            "ALLOY_SERVER_ADDR",
        ] {
            env::remove_var(key);
        }
    }

    #[test]
    fn dual_port_requires_both_addresses() {
        let err = OpenportioServer::new()
            .with_rest_addr(SocketAddr::from(([127, 0, 0, 1], 4100)))
            .dual_port_addrs()
            .expect_err("single dual-port addr should fail");
        assert!(err
            .to_string()
            .contains("requires both with_rest_addr(...) and with_grpc_addr(...)"));
    }
}
