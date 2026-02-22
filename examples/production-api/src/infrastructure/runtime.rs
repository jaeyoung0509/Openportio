use std::time::Duration;

use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

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

pub(crate) fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
