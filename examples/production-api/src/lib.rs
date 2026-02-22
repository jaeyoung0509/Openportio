use std::{error::Error, sync::Arc};

use openportio_core::AppState;
use openportio_server::{
    auth::AuthRuntimeConfig,
    grpc::{GrpcHandlerContext, GrpcHelloRequest, GrpcHelloResponse},
    OpenportioServer,
};
use sqlx::postgres::PgPoolOptions;

pub(crate) mod application;
pub(crate) mod domain;
pub(crate) mod infrastructure;
pub(crate) mod presentation;

#[cfg(test)]
mod tests;

fn adapt_shared_greeting_for_grpc(
    use_case: Arc<
        application::greeting::GreetingUseCase<application::greeting::ServiceGreetingAdapter>,
    >,
    context: GrpcHandlerContext,
    request: GrpcHelloRequest,
) -> Result<GrpcHelloResponse, openportio_core::OpenportioError> {
    let result = use_case.execute(domain::greeting::GreetingCommand {
        name: request.name,
        actor: Some(context.principal().subject.clone()),
        channel: domain::greeting::GreetingChannel::Grpc,
    })?;
    Ok(GrpcHelloResponse {
        message: result.message,
    })
}

pub async fn run() -> Result<(), Box<dyn Error>> {
    let _observability = infrastructure::runtime::init_observability()
        .map_err(|err| format!("failed to initialize observability: {err}"))?;
    let config = infrastructure::config::ProductionConfig::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(config.max_db_connections)
        .connect_lazy(&config.database_url)?;

    if config.run_migrations {
        infrastructure::runtime::spawn_migration_worker(
            pool.clone(),
            config.migration_retry_seconds,
        );
    }

    let rest_state = Arc::new(infrastructure::state::ProductionApiState::new(
        config.service_name.clone(),
        pool,
    ));
    let shared_greeting_use_case = rest_state.greeting_use_case.clone();
    let rest_router = presentation::router::build_rest_router(
        rest_state,
        AuthRuntimeConfig::from_env(),
        config.enable_drill_routes,
    );

    let grpc_state = Arc::new(AppState::local(config.service_name.clone()));
    OpenportioServer::new()
        .with_addr(config.addr)
        .with_state(grpc_state)
        .with_grpc_say_hello_with_context(move |context, request| {
            let use_case = shared_greeting_use_case.clone();
            async move { adapt_shared_greeting_for_grpc(use_case, context, request) }
        })
        .with_rest_router(rest_router)
        .on_startup(|addr| {
            tracing::info!(addr = %addr, "production-api started");
        })
        .on_shutdown(|| {
            tracing::info!("production-api shutting down");
        })
        .run()
        .await?;

    Ok(())
}
