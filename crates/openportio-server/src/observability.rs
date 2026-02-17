use std::{env, str::FromStr, time::Duration};

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace::Sampler, Resource};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const DEFAULT_METRICS_PATH: &str = "/metrics";
const DEFAULT_OTEL_TIMEOUT_SECONDS: u64 = 3;
const DEFAULT_OTEL_TRACE_SAMPLE_RATIO: f64 = 1.0;
const DEFAULT_SERVICE_NAME: &str = "openportio-server";
const DEFAULT_LOG_FILTER: &str = "info,openportio_server=info,tower_http=info";

#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    pub service_name: String,
    pub log_filter: String,
    pub metrics_path: String,
    pub otel_exporter_otlp_endpoint: Option<String>,
    pub otel_exporter_timeout_seconds: u64,
    pub otel_trace_sample_ratio: f64,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            service_name: DEFAULT_SERVICE_NAME.to_string(),
            log_filter: DEFAULT_LOG_FILTER.to_string(),
            metrics_path: DEFAULT_METRICS_PATH.to_string(),
            otel_exporter_otlp_endpoint: None,
            otel_exporter_timeout_seconds: DEFAULT_OTEL_TIMEOUT_SECONDS,
            otel_trace_sample_ratio: DEFAULT_OTEL_TRACE_SAMPLE_RATIO,
        }
    }
}

impl ObservabilityConfig {
    pub fn from_env() -> Self {
        let service_name = read_env_string_with_aliases(&[
            "OPENPORTIO_SERVICE_NAME",
            "MELD_SERVICE_NAME",
            "ALLOY_SERVICE_NAME",
        ])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string());

        let log_filter = read_env_string_with_aliases(&[
            "RUST_LOG",
            "OPENPORTIO_LOG_FILTER",
            "MELD_LOG_FILTER",
            "ALLOY_LOG_FILTER",
        ])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_string());

        let metrics_path = read_env_string_with_aliases(&[
            "OPENPORTIO_METRICS_PATH",
            "MELD_METRICS_PATH",
            "ALLOY_METRICS_PATH",
        ])
        .unwrap_or_else(|| DEFAULT_METRICS_PATH.to_string());
        let metrics_path = normalize_metrics_path(&metrics_path);

        let otel_exporter_otlp_endpoint = read_env_string_with_aliases(&[
            "OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT",
            "MELD_OTEL_EXPORTER_OTLP_ENDPOINT",
            "ALLOY_OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
        ])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

        let otel_exporter_timeout_seconds = read_env_with_aliases(&[
            "OPENPORTIO_OTEL_EXPORTER_TIMEOUT_SECONDS",
            "MELD_OTEL_EXPORTER_TIMEOUT_SECONDS",
            "ALLOY_OTEL_EXPORTER_TIMEOUT_SECONDS",
        ])
        .unwrap_or(DEFAULT_OTEL_TIMEOUT_SECONDS);

        let otel_trace_sample_ratio = read_env_with_aliases(&[
            "OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO",
            "MELD_OTEL_TRACE_SAMPLE_RATIO",
            "ALLOY_OTEL_TRACE_SAMPLE_RATIO",
        ])
        .unwrap_or(DEFAULT_OTEL_TRACE_SAMPLE_RATIO)
        .clamp(0.0, 1.0);

        Self {
            service_name,
            log_filter,
            metrics_path,
            otel_exporter_otlp_endpoint,
            otel_exporter_timeout_seconds,
            otel_trace_sample_ratio,
        }
    }

    pub fn otel_enabled(&self) -> bool {
        self.otel_exporter_otlp_endpoint.is_some()
    }
}

#[derive(Debug)]
pub struct ObservabilityGuard {
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown();
        }
    }
}

pub fn init_observability_from_env() -> Result<ObservabilityGuard, String> {
    init_observability(&ObservabilityConfig::from_env())
}

pub fn init_observability(config: &ObservabilityConfig) -> Result<ObservabilityGuard, String> {
    let env_filter = EnvFilter::try_new(config.log_filter.clone())
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true);

    if let Some(endpoint) = config.otel_exporter_otlp_endpoint.as_ref() {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.clone())
            .with_timeout(Duration::from_secs(config.otel_exporter_timeout_seconds))
            .build()
            .map_err(|err| format!("failed to initialize OTel exporter: {err}"))?;

        let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_sampler(Sampler::TraceIdRatioBased(config.otel_trace_sample_ratio))
            .with_resource(
                Resource::builder_empty()
                    .with_attributes([KeyValue::new("service.name", config.service_name.clone())])
                    .build(),
            )
            .build();
        global::set_tracer_provider(tracer_provider.clone());
        let tracer = tracer_provider.tracer(config.service_name.clone());

        let subscriber_result = tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .try_init();
        handle_subscriber_result(subscriber_result)?;

        tracing::info!(
            endpoint = %endpoint,
            sample_ratio = config.otel_trace_sample_ratio,
            service_name = %config.service_name,
            "observability initialized with OTLP exporter"
        );

        return Ok(ObservabilityGuard {
            tracer_provider: Some(tracer_provider),
        });
    }

    let subscriber_result = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();
    handle_subscriber_result(subscriber_result)?;

    tracing::info!(
        metrics_path = %config.metrics_path,
        "observability initialized with logging + middleware metrics"
    );

    Ok(ObservabilityGuard {
        tracer_provider: None,
    })
}

fn handle_subscriber_result(
    result: Result<(), tracing_subscriber::util::TryInitError>,
) -> Result<(), String> {
    let Err(error) = result else {
        return Ok(());
    };
    let message = error.to_string();
    if message.contains("global default trace dispatcher has already been set") {
        return Ok(());
    }
    Err(message)
}

fn normalize_metrics_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_METRICS_PATH.to_string();
    }

    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };

    let is_valid = normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '/' || ch == '-' || ch == '_' || ch == '.');
    if !is_valid {
        return DEFAULT_METRICS_PATH.to_string();
    }

    normalized
}

fn read_env<T>(name: &str) -> Option<T>
where
    T: FromStr,
{
    env::var(name).ok().and_then(|raw| raw.parse::<T>().ok())
}

fn read_env_with_aliases<T>(names: &[&str]) -> Option<T>
where
    T: FromStr,
{
    names.iter().find_map(|name| read_env(name))
}

fn read_env_string_with_aliases(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn defaults_are_production_safe() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        clear_observability_env();

        let config = ObservabilityConfig::from_env();
        assert_eq!(config.metrics_path, "/metrics");
        assert!(!config.otel_enabled());
        assert_eq!(config.otel_trace_sample_ratio, 1.0);
        assert_eq!(config.otel_exporter_timeout_seconds, 3);
    }

    #[test]
    fn metrics_path_is_normalized_and_aliases_are_supported() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        clear_observability_env();

        env::set_var("MELD_METRICS_PATH", "telemetry");
        env::set_var("ALLOY_SERVICE_NAME", "legacy-service");
        env::set_var("MELD_OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4317");
        env::set_var("MELD_OTEL_TRACE_SAMPLE_RATIO", "3.5");

        let config = ObservabilityConfig::from_env();
        assert_eq!(config.metrics_path, "/telemetry");
        assert_eq!(config.service_name, "legacy-service");
        assert_eq!(
            config.otel_exporter_otlp_endpoint.as_deref(),
            Some("http://127.0.0.1:4317")
        );
        assert_eq!(config.otel_trace_sample_ratio, 1.0);

        clear_observability_env();
    }

    #[test]
    fn invalid_metrics_path_falls_back_to_default() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        clear_observability_env();

        env::set_var("OPENPORTIO_METRICS_PATH", "bad path with spaces");
        let config = ObservabilityConfig::from_env();
        assert_eq!(config.metrics_path, "/metrics");

        clear_observability_env();
    }

    fn clear_observability_env() {
        for key in [
            "RUST_LOG",
            "OPENPORTIO_LOG_FILTER",
            "OPENPORTIO_SERVICE_NAME",
            "OPENPORTIO_METRICS_PATH",
            "OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT",
            "OPENPORTIO_OTEL_EXPORTER_TIMEOUT_SECONDS",
            "OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO",
            "MELD_LOG_FILTER",
            "MELD_SERVICE_NAME",
            "MELD_METRICS_PATH",
            "MELD_OTEL_EXPORTER_OTLP_ENDPOINT",
            "MELD_OTEL_EXPORTER_TIMEOUT_SECONDS",
            "MELD_OTEL_TRACE_SAMPLE_RATIO",
            "ALLOY_LOG_FILTER",
            "ALLOY_SERVICE_NAME",
            "ALLOY_METRICS_PATH",
            "ALLOY_OTEL_EXPORTER_OTLP_ENDPOINT",
            "ALLOY_OTEL_EXPORTER_TIMEOUT_SECONDS",
            "ALLOY_OTEL_TRACE_SAMPLE_RATIO",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
        ] {
            env::remove_var(key);
        }
    }
}
