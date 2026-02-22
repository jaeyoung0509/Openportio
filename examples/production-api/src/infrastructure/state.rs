use std::sync::Arc;

use sqlx::PgPool;

use crate::application::greeting::{GreetingUseCase, ServiceGreetingAdapter};

#[derive(Clone)]
pub(crate) struct ProductionApiState {
    pub(crate) service_name: String,
    pub(crate) pool: PgPool,
    pub(crate) greeting_use_case: Arc<GreetingUseCase<ServiceGreetingAdapter>>,
}

impl ProductionApiState {
    pub(crate) fn new(service_name: String, pool: PgPool) -> Self {
        let greeting_use_case = Arc::new(GreetingUseCase::new(ServiceGreetingAdapter::new(
            service_name.clone(),
        )));
        Self {
            service_name,
            pool,
            greeting_use_case,
        }
    }
}
