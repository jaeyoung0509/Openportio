use std::{error::Error, sync::Arc};

use openportio_core::AppState;
use openportio_server::{auth::AuthRuntimeConfig, OpenportioServer};
use sqlx::postgres::PgPoolOptions;

pub(crate) mod application;
pub(crate) mod domain;
pub(crate) mod infrastructure;
pub(crate) mod presentation;

#[cfg(test)]
mod tests;

pub async fn run() -> Result<(), Box<dyn Error>> {
    infrastructure::runtime::init_tracing();
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
    let rest_router = presentation::router::build_rest_router(
        rest_state,
        AuthRuntimeConfig::from_env(),
        config.enable_drill_routes,
    );

    let grpc_state = Arc::new(AppState::local(config.service_name.clone()));
    OpenportioServer::new()
        .with_addr(config.addr)
        .with_state(grpc_state)
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
