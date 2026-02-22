use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct ProductionApiState {
    pub(crate) service_name: String,
    pub(crate) pool: PgPool,
}

impl ProductionApiState {
    pub(crate) fn new(service_name: String, pool: PgPool) -> Self {
        Self { service_name, pool }
    }
}
