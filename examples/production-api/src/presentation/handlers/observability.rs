use axum::http::header;

pub(crate) async fn metrics() -> ([(header::HeaderName, &'static str); 1], String) {
    (
        [(
            header::CONTENT_TYPE,
            openportio_server::middleware::metrics_content_type(),
        )],
        openportio_server::middleware::render_prometheus_metrics(),
    )
}
