use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct StatusResponse {
    pub(crate) status: &'static str,
}

#[derive(Debug, Serialize)]
pub(crate) struct HealthResponse {
    pub(crate) status: &'static str,
    pub(crate) service: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReadyResponse {
    pub(crate) status: &'static str,
    pub(crate) database: &'static str,
}
