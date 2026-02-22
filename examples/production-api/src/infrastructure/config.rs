use std::{env, error::Error, fmt, net::SocketAddr, str::FromStr};

#[derive(Debug, Clone)]
pub(crate) struct ProductionConfig {
    pub(crate) addr: SocketAddr,
    pub(crate) service_name: String,
    pub(crate) database_url: String,
    pub(crate) max_db_connections: u32,
    pub(crate) run_migrations: bool,
    pub(crate) migration_retry_seconds: u64,
    pub(crate) enable_drill_routes: bool,
}

#[derive(Debug)]
pub(crate) enum ConfigError {
    MissingRequired {
        key: &'static str,
    },
    InvalidValue {
        key: &'static str,
        value: String,
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequired { key } => {
                write!(f, "missing required environment variable: {key}")
            }
            Self::InvalidValue { key, value, reason } => {
                write!(f, "invalid value for {key}='{value}': {reason}")
            }
        }
    }
}

impl Error for ConfigError {}

impl ProductionConfig {
    pub(crate) fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub(crate) fn from_lookup<F>(mut lookup: F) -> Result<Self, ConfigError>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let addr = parse_or_default(
            &mut lookup,
            "PROD_API_ADDR",
            SocketAddr::from(([127, 0, 0, 1], 4100)),
        )?;

        let service_name = lookup("PROD_API_SERVICE_NAME")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "production-api".to_string());

        let database_url = required_value(&mut lookup, "PROD_API_DATABASE_URL")?;
        let max_db_connections = parse_or_default(&mut lookup, "PROD_API_DB_MAX_CONNECTIONS", 10)?;
        let run_migrations = parse_or_default(&mut lookup, "PROD_API_RUN_MIGRATIONS", true)?;
        let migration_retry_seconds =
            parse_or_default(&mut lookup, "PROD_API_MIGRATION_RETRY_SECONDS", 5)?;
        let enable_drill_routes =
            parse_or_default(&mut lookup, "PROD_API_ENABLE_DRILL_ROUTES", false)?;

        Ok(Self {
            addr,
            service_name,
            database_url,
            max_db_connections,
            run_migrations,
            migration_retry_seconds,
            enable_drill_routes,
        })
    }
}

fn required_value<F>(lookup: &mut F, key: &'static str) -> Result<String, ConfigError>
where
    F: FnMut(&str) -> Option<String>,
{
    match lookup(key).map(|value| value.trim().to_string()) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(ConfigError::MissingRequired { key }),
    }
}

fn parse_or_default<T, F>(lookup: &mut F, key: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
    F: FnMut(&str) -> Option<String>,
{
    match lookup(key) {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(ConfigError::InvalidValue {
                    key,
                    value: raw,
                    reason: "value must not be empty".to_string(),
                });
            }

            trimmed
                .parse::<T>()
                .map_err(|err| ConfigError::InvalidValue {
                    key,
                    value: raw,
                    reason: err.to_string(),
                })
        }
        None => Ok(default),
    }
}
