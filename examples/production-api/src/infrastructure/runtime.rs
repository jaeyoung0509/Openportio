use std::time::Duration;

use openportio_server::observability::ObservabilityGuard;
use sqlx::PgPool;

pub(crate) fn spawn_migration_worker(pool: PgPool, retry_seconds: u64) {
    tokio::spawn(async move {
        let retry_interval = Duration::from_secs(retry_seconds.max(1));

        loop {
            match sqlx::migrate!("./migrations").run(&pool).await {
                Ok(_) => {
                    tracing::info!("database migrations are up to date");
                    break;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        retry_seconds = retry_interval.as_secs(),
                        "migration failed, retrying"
                    );
                    tokio::time::sleep(retry_interval).await;
                }
            }
        }
    });
}

pub(crate) fn init_observability() -> Result<ObservabilityGuard, String> {
    openportio_server::observability::init_observability_from_env()
}
