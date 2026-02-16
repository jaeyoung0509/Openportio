use std::{
    collections::HashMap,
    env,
    io::Read,
    str::FromStr,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode_header, jwk::JwkSet, Algorithm, DecodingKey};
use openportio_core::auth::{
    validate_bearer_jwt, validate_bearer_jwt_with_key, AuthPrincipal, JwtValidationConfig,
};
use tonic::Status;

use crate::api::ApiErrorResponse;

const DEFAULT_JWKS_REFRESH_SECS: u64 = 300;
const DEFAULT_JWKS_CONNECT_TIMEOUT_SECS: u64 = 2;
const DEFAULT_JWKS_IO_TIMEOUT_SECS: u64 = 5;
const MAX_JWKS_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug)]
struct JwksProvider {
    url: String,
    refresh_interval: Duration,
    allowed_algorithms: Vec<Algorithm>,
    client: ureq::Agent,
    refresh_lock: Mutex<()>,
    state: RwLock<JwksState>,
}

#[derive(Debug, Default)]
struct JwksState {
    keys: HashMap<String, DecodingKey>,
    last_refresh: Option<Instant>,
}

impl JwksProvider {
    fn new(url: String, refresh_secs: u64, allowed_algorithms: Vec<Algorithm>) -> Self {
        Self {
            url,
            refresh_interval: Duration::from_secs(refresh_secs.max(1)),
            allowed_algorithms,
            client: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(DEFAULT_JWKS_CONNECT_TIMEOUT_SECS))
                .timeout_read(Duration::from_secs(DEFAULT_JWKS_IO_TIMEOUT_SECS))
                .timeout_write(Duration::from_secs(DEFAULT_JWKS_IO_TIMEOUT_SECS))
                .build(),
            refresh_lock: Mutex::new(()),
            state: RwLock::new(JwksState::default()),
        }
    }

    fn decoding_key_for_token(
        &self,
        token: &str,
    ) -> Result<(DecodingKey, Algorithm), AuthRejection> {
        let header = decode_header(token)
            .map_err(|err| AuthRejection::InvalidToken(format!("invalid token header: {err}")))?;
        let kid = header.kid.ok_or_else(|| {
            AuthRejection::InvalidToken("token header missing kid for jwks validation".to_string())
        })?;
        let algorithm = header.alg;

        if !self.allowed_algorithms.contains(&algorithm) {
            return Err(AuthRejection::InvalidToken(format!(
                "algorithm {:?} is not allowed in jwks mode",
                algorithm
            )));
        }

        self.refresh_if_needed()?;
        if let Some(key) = self.cached_key(&kid)? {
            return Ok((key, algorithm));
        }

        // Do not force an immediate network refresh for untrusted kid values.
        let key = self.cached_key(&kid)?.ok_or_else(|| {
            AuthRejection::InvalidToken(format!(
                "unknown jwks key id `{kid}` (will retry on next refresh interval)"
            ))
        })?;

        Ok((key, algorithm))
    }

    fn refresh_if_needed(&self) -> Result<(), AuthRejection> {
        if !self.should_refresh()? {
            return Ok(());
        }

        let _refresh_guard = self
            .refresh_lock
            .lock()
            .map_err(|_| AuthRejection::Misconfigured("jwks refresh lock poisoned".to_string()))?;

        // Double-check after obtaining refresh lock to prevent thundering herd refreshes.
        if !self.should_refresh()? {
            return Ok(());
        }

        match self.refresh_keys() {
            Ok(()) => Ok(()),
            Err(err) => {
                if self.has_cached_keys()? {
                    tracing::warn!(error = ?err, "jwks refresh failed; continuing with cached keys");
                    Ok(())
                } else {
                    Err(err)
                }
            }
        }
    }

    fn should_refresh(&self) -> Result<bool, AuthRejection> {
        let guard = self
            .state
            .read()
            .map_err(|_| AuthRejection::Misconfigured("jwks cache lock poisoned".to_string()))?;
        let now = Instant::now();
        let should_refresh = match guard.last_refresh {
            Some(last) => now.duration_since(last) >= self.refresh_interval,
            None => true,
        };
        Ok(should_refresh)
    }

    fn has_cached_keys(&self) -> Result<bool, AuthRejection> {
        let guard = self
            .state
            .read()
            .map_err(|_| AuthRejection::Misconfigured("jwks cache lock poisoned".to_string()))?;
        Ok(!guard.keys.is_empty())
    }

    fn cached_key(&self, kid: &str) -> Result<Option<DecodingKey>, AuthRejection> {
        let guard = self
            .state
            .read()
            .map_err(|_| AuthRejection::Misconfigured("jwks cache lock poisoned".to_string()))?;
        Ok(guard.keys.get(kid).cloned())
    }

    fn refresh_keys(&self) -> Result<(), AuthRejection> {
        let jwk_set = self.fetch_jwks()?;
        let mut keys = HashMap::new();

        for jwk in jwk_set.keys {
            let Some(kid) = jwk.common.key_id.clone() else {
                continue;
            };
            match DecodingKey::from_jwk(&jwk) {
                Ok(key) => {
                    keys.insert(kid, key);
                }
                Err(err) => {
                    tracing::warn!(kid = %kid, error = %err, "failed to parse jwk key; skipping");
                }
            }
        }

        if keys.is_empty() {
            return Err(AuthRejection::Misconfigured(
                "jwks payload contains no usable keys".to_string(),
            ));
        }

        let mut guard = self
            .state
            .write()
            .map_err(|_| AuthRejection::Misconfigured("jwks cache lock poisoned".to_string()))?;
        guard.keys = keys;
        guard.last_refresh = Some(Instant::now());
        Ok(())
    }

    fn fetch_jwks(&self) -> Result<JwkSet, AuthRejection> {
        self.run_blocking_fetch()
    }

    fn run_blocking_fetch(&self) -> Result<JwkSet, AuthRejection> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) {
                return tokio::task::block_in_place(|| self.fetch_jwks_blocking());
            }
        }

        self.fetch_jwks_blocking()
    }

    fn fetch_jwks_blocking(&self) -> Result<JwkSet, AuthRejection> {
        let response = match self.client.get(&self.url).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _response)) => {
                return Err(AuthRejection::Misconfigured(format!(
                    "jwks endpoint returned {status}"
                )));
            }
            Err(err) => {
                return Err(AuthRejection::Misconfigured(format!(
                    "failed to fetch jwks: {err}"
                )));
            }
        };

        let mut limited = response
            .into_reader()
            .take((MAX_JWKS_RESPONSE_BYTES + 1) as u64);
        let mut bytes = Vec::new();
        limited.read_to_end(&mut bytes).map_err(|err| {
            AuthRejection::Misconfigured(format!("failed to read jwks body: {err}"))
        })?;

        if bytes.len() > MAX_JWKS_RESPONSE_BYTES {
            return Err(AuthRejection::Misconfigured(format!(
                "jwks payload exceeds max size of {MAX_JWKS_RESPONSE_BYTES} bytes"
            )));
        }

        let raw = String::from_utf8(bytes).map_err(|err| {
            AuthRejection::Misconfigured(format!("jwks payload is not valid utf-8: {err}"))
        })?;

        serde_json::from_str::<JwkSet>(&raw)
            .map_err(|err| AuthRejection::Misconfigured(format!("invalid jwks payload: {err}")))
    }
}

#[derive(Debug, Clone)]
pub struct AuthRuntimeConfig {
    pub enabled: bool,
    pub jwt_secret: Option<String>,
    pub jwks_url: Option<String>,
    pub jwks_refresh_secs: u64,
    pub jwks_allowed_algorithms: Vec<Algorithm>,
    pub expected_issuer: Option<String>,
    pub expected_audience: Option<String>,
    jwks_provider: Option<Arc<JwksProvider>>,
}

impl Default for AuthRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            jwt_secret: None,
            jwks_url: None,
            jwks_refresh_secs: DEFAULT_JWKS_REFRESH_SECS,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: None,
            expected_audience: None,
            jwks_provider: None,
        }
    }
}

impl AuthRuntimeConfig {
    pub fn from_env() -> Self {
        let enabled = read_env_bool_with_aliases(&[
            "OPENPORTIO_AUTH_ENABLED",
            "MELD_AUTH_ENABLED",
            "ALLOY_AUTH_ENABLED",
        ])
        .unwrap_or(false);
        let jwt_secret = read_env_string_with_aliases(&[
            "OPENPORTIO_AUTH_JWT_SECRET",
            "MELD_AUTH_JWT_SECRET",
            "ALLOY_AUTH_JWT_SECRET",
        ]);
        let jwks_url = read_env_string_with_aliases(&[
            "OPENPORTIO_AUTH_JWKS_URL",
            "MELD_AUTH_JWKS_URL",
            "ALLOY_AUTH_JWKS_URL",
        ]);
        let jwks_refresh_secs = read_env_u64_with_aliases(&[
            "OPENPORTIO_AUTH_JWKS_REFRESH_SECS",
            "MELD_AUTH_JWKS_REFRESH_SECS",
            "ALLOY_AUTH_JWKS_REFRESH_SECS",
        ])
        .unwrap_or(DEFAULT_JWKS_REFRESH_SECS);
        let jwks_allowed_algorithms = read_env_algorithms_with_aliases(&[
            "OPENPORTIO_AUTH_JWKS_ALGORITHMS",
            "MELD_AUTH_JWKS_ALGORITHMS",
            "ALLOY_AUTH_JWKS_ALGORITHMS",
        ])
        .unwrap_or_else(default_jwks_algorithms);

        let cfg = Self {
            enabled,
            jwt_secret,
            jwks_url: jwks_url.clone(),
            jwks_refresh_secs,
            jwks_allowed_algorithms: jwks_allowed_algorithms.clone(),
            expected_issuer: read_env_string_with_aliases(&[
                "OPENPORTIO_AUTH_ISSUER",
                "MELD_AUTH_ISSUER",
                "ALLOY_AUTH_ISSUER",
            ]),
            expected_audience: read_env_string_with_aliases(&[
                "OPENPORTIO_AUTH_AUDIENCE",
                "MELD_AUTH_AUDIENCE",
                "ALLOY_AUTH_AUDIENCE",
            ]),
            jwks_provider: jwks_url.map(|url| {
                Arc::new(JwksProvider::new(
                    url,
                    jwks_refresh_secs,
                    jwks_allowed_algorithms,
                ))
            }),
        };

        if let Some(provider) = cfg.jwks_provider.as_ref() {
            if let Err(err) = provider.refresh_keys() {
                tracing::warn!(
                    error = ?err,
                    "initial jwks fetch failed; runtime will retry during authentication"
                );
            }
        }

        cfg
    }

    fn jwt_validation_config(&self) -> Result<JwtValidationConfig, AuthRejection> {
        let secret = self.jwt_secret.clone().ok_or_else(|| {
            AuthRejection::Misconfigured(
                "OPENPORTIO_AUTH_JWT_SECRET is missing (or configure OPENPORTIO_AUTH_JWKS_URL)"
                    .to_string(),
            )
        })?;

        Ok(JwtValidationConfig {
            secret,
            expected_issuer: self.expected_issuer.clone(),
            expected_audience: self.expected_audience.clone(),
        })
    }

    pub fn authenticate_authorization_value_str(
        &self,
        auth_value: &str,
    ) -> Result<AuthPrincipal, AuthRejection> {
        if !self.enabled {
            return Ok(AuthPrincipal {
                subject: "anonymous".to_string(),
                issuer: None,
                audience: vec![],
                scopes: vec![],
            });
        }

        let token = parse_bearer_token(auth_value)?;

        if let Some(provider) = &self.jwks_provider {
            let (decoding_key, algorithm) = provider.decoding_key_for_token(token)?;
            return validate_bearer_jwt_with_key(
                token,
                &decoding_key,
                algorithm,
                self.expected_issuer.as_deref(),
                self.expected_audience.as_deref(),
            )
            .map_err(|err| AuthRejection::InvalidToken(err.to_string()));
        }

        let validation_cfg = self.jwt_validation_config()?;
        validate_bearer_jwt(token, &validation_cfg)
            .map_err(|err| AuthRejection::InvalidToken(err.to_string()))
    }

    pub fn authenticate_header_value(
        &self,
        auth_value: Option<&HeaderValue>,
    ) -> Result<AuthPrincipal, AuthRejection> {
        if !self.enabled {
            return Ok(AuthPrincipal {
                subject: "anonymous".to_string(),
                issuer: None,
                audience: vec![],
                scopes: vec![],
            });
        }

        let value = auth_value
            .ok_or(AuthRejection::MissingAuthorization)?
            .to_str()
            .map_err(|_| {
                AuthRejection::InvalidToken("authorization header is invalid".to_string())
            })?;

        self.authenticate_authorization_value_str(value)
    }

    pub fn authenticate_headers(
        &self,
        headers: &HeaderMap,
    ) -> Result<AuthPrincipal, AuthRejection> {
        self.authenticate_header_value(headers.get(header::AUTHORIZATION))
    }
}

#[derive(Debug, Clone)]
pub enum AuthRejection {
    MissingAuthorization,
    InvalidToken(String),
    Misconfigured(String),
}

impl AuthRejection {
    pub fn into_rest_response(self) -> Response {
        match self {
            Self::MissingAuthorization => (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    code: "unauthorized".to_string(),
                    message: "missing bearer token".to_string(),
                    detail: None,
                    details: None,
                }),
            )
                .into_response(),
            Self::InvalidToken(message) => (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    code: "unauthorized".to_string(),
                    message,
                    detail: None,
                    details: None,
                }),
            )
                .into_response(),
            Self::Misconfigured(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResponse {
                    code: "internal_error".to_string(),
                    message,
                    detail: None,
                    details: None,
                }),
            )
                .into_response(),
        }
    }

    pub fn into_grpc_status(self) -> Status {
        match self {
            Self::MissingAuthorization => Status::unauthenticated("missing bearer token"),
            Self::InvalidToken(message) => Status::unauthenticated(message),
            Self::Misconfigured(message) => Status::internal(message),
        }
    }
}

pub async fn rest_auth_middleware(
    State(cfg): State<AuthRuntimeConfig>,
    mut req: Request,
    next: Next,
) -> Response {
    match cfg.authenticate_headers(req.headers()) {
        Ok(principal) => {
            req.extensions_mut().insert(principal);
            next.run(req).await
        }
        Err(rejection) => rejection.into_rest_response(),
    }
}

pub fn parse_bearer_token(value: &str) -> Result<&str, AuthRejection> {
    let mut parts = value.splitn(2, ' ');
    let scheme = parts.next().unwrap_or_default();
    let token = parts.next().unwrap_or_default();

    if !scheme.eq_ignore_ascii_case("bearer") || token.trim().is_empty() {
        return Err(AuthRejection::InvalidToken(
            "authorization header must be Bearer <token>".to_string(),
        ));
    }

    Ok(token.trim())
}

fn default_jwks_algorithms() -> Vec<Algorithm> {
    vec![
        Algorithm::RS256,
        Algorithm::RS384,
        Algorithm::RS512,
        Algorithm::ES256,
        Algorithm::ES384,
    ]
}

fn parse_jwks_algorithm(raw: &str) -> Option<Algorithm> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "RS256" => Some(Algorithm::RS256),
        "RS384" => Some(Algorithm::RS384),
        "RS512" => Some(Algorithm::RS512),
        "ES256" => Some(Algorithm::ES256),
        "ES384" => Some(Algorithm::ES384),
        _ => None,
    }
}

fn read_env_bool(name: &str) -> Option<bool> {
    env::var(name)
        .ok()
        .and_then(|raw| bool::from_str(raw.trim()).ok())
}

fn read_env_bool_with_aliases(names: &[&str]) -> Option<bool> {
    names.iter().find_map(|name| read_env_bool(name))
}

fn read_env_string_with_aliases(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| env::var(name).ok())
}

fn read_env_u64_with_aliases(names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| env::var(name).ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

fn read_env_algorithms_with_aliases(names: &[&str]) -> Option<Vec<Algorithm>> {
    let raw = names.iter().find_map(|name| env::var(name).ok())?;
    let mut parsed = Vec::new();

    for entry in raw.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        match parse_jwks_algorithm(trimmed) {
            Some(algorithm) => {
                if !parsed.contains(&algorithm) {
                    parsed.push(algorithm);
                }
            }
            None => {
                tracing::warn!(algorithm = %trimmed, "ignoring unsupported jwks algorithm entry");
            }
        }
    }

    if parsed.is_empty() {
        tracing::warn!(
            "OPENPORTIO_AUTH_JWKS_ALGORITHMS was set but contained no supported entries"
        );
        None
    } else {
        Some(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{
        encode,
        jwk::{Jwk, JwkSet},
        EncodingKey, Header,
    };
    use openportio_core::auth::{AudienceClaim, JwtClaims};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, LazyLock, Mutex,
        },
        thread,
    };

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    const TEST_RSA_PRIVATE_KEY_DER: &[u8] = include_bytes!("../tests/fixtures/private_rsa_key.der");

    #[test]
    fn from_env_supports_meld_compatibility_aliases() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        clear_auth_env();

        env::set_var("MELD_AUTH_ENABLED", "true");
        env::set_var("MELD_AUTH_JWT_SECRET", "legacy-secret");
        env::set_var("MELD_AUTH_JWKS_URL", "https://legacy.example/jwks");
        env::set_var("MELD_AUTH_JWKS_REFRESH_SECS", "600");
        env::set_var("MELD_AUTH_JWKS_ALGORITHMS", "RS256,ES256");
        env::set_var("MELD_AUTH_ISSUER", "https://issuer.legacy");
        env::set_var("MELD_AUTH_AUDIENCE", "legacy-audience");

        let cfg = AuthRuntimeConfig::from_env();
        assert!(cfg.enabled);
        assert_eq!(cfg.jwt_secret.as_deref(), Some("legacy-secret"));
        assert_eq!(cfg.jwks_url.as_deref(), Some("https://legacy.example/jwks"));
        assert_eq!(cfg.jwks_refresh_secs, 600);
        assert!(cfg.jwks_allowed_algorithms.contains(&Algorithm::RS256));
        assert!(cfg.jwks_allowed_algorithms.contains(&Algorithm::ES256));
        assert_eq!(
            cfg.expected_issuer.as_deref(),
            Some("https://issuer.legacy")
        );
        assert_eq!(cfg.expected_audience.as_deref(), Some("legacy-audience"));

        clear_auth_env();
    }

    #[test]
    fn jwks_mode_validates_rs256_token() {
        let jwks_body = build_jwks_json("rsa-key-1");
        let (jwks_url, _payload, _request_count, shutdown_tx) = spawn_jwks_server(jwks_body);

        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 300,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                300,
                default_jwks_algorithms(),
            ))),
        };

        let token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        let principal = cfg
            .authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect("jwks token should validate");

        assert_eq!(principal.subject, "user-1");
        assert_eq!(principal.issuer.as_deref(), Some("https://issuer.local"));
        assert!(principal.audience.iter().any(|aud| aud == "openportio-api"));

        let _ = shutdown_tx.send(());
    }

    #[test]
    fn jwks_refresh_failure_uses_cached_keys() {
        let jwks_body = build_jwks_json("rsa-key-1");
        let (jwks_url, payload, _request_count, shutdown_tx) = spawn_jwks_server(jwks_body);

        let provider = Arc::new(JwksProvider::new(
            jwks_url.clone(),
            1,
            default_jwks_algorithms(),
        ));
        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url),
            jwks_refresh_secs: 1,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::clone(&provider)),
        };

        let token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        cfg.authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect("initial validation should work");

        {
            let mut guard = payload.lock().expect("payload lock");
            *guard = "{ invalid-json".to_string();
        }

        std::thread::sleep(Duration::from_millis(1100));

        let principal = cfg
            .authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect("cached key should survive jwks refresh failure");
        assert_eq!(principal.subject, "user-1");

        let _ = shutdown_tx.send(());
    }

    #[test]
    fn jwks_rejects_unknown_kid() {
        let jwks_body = build_jwks_json("rsa-key-1");
        let (jwks_url, _payload, _request_count, shutdown_tx) = spawn_jwks_server(jwks_body);

        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 300,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                300,
                default_jwks_algorithms(),
            ))),
        };

        let token = build_rs256_token("unknown-key", "https://issuer.local", "openportio-api");
        let err = cfg
            .authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect_err("unknown kid must fail");

        match err {
            AuthRejection::InvalidToken(message) => {
                assert!(message.contains("unknown jwks key id"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = shutdown_tx.send(());
    }

    #[test]
    fn jwks_unknown_kid_does_not_force_immediate_refresh() {
        let jwks_body = build_jwks_json("rsa-key-1");
        let (jwks_url, _payload, request_count, shutdown_tx) = spawn_jwks_server(jwks_body);

        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 300,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                300,
                default_jwks_algorithms(),
            ))),
        };

        let known_token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        cfg.authenticate_authorization_value_str(&format!("Bearer {known_token}"))
            .expect("known kid should validate");
        let before_unknown = request_count.load(Ordering::SeqCst);

        let unknown_token =
            build_rs256_token("unknown-key", "https://issuer.local", "openportio-api");
        let err = cfg
            .authenticate_authorization_value_str(&format!("Bearer {unknown_token}"))
            .expect_err("unknown kid must fail");
        assert!(matches!(err, AuthRejection::InvalidToken(_)));

        let after_unknown = request_count.load(Ordering::SeqCst);
        assert_eq!(
            before_unknown, after_unknown,
            "unknown kid must not trigger an out-of-interval jwks refresh"
        );

        let _ = shutdown_tx.send(());
    }

    #[test]
    fn jwks_concurrent_refresh_performs_single_fetch() {
        let jwks_body = build_jwks_json("rsa-key-1");
        let (jwks_url, _payload, request_count, shutdown_tx) = spawn_jwks_server(jwks_body);

        let cfg = Arc::new(AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 1,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                1,
                default_jwks_algorithms(),
            ))),
        });

        let token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        let auth_header = format!("Bearer {token}");
        cfg.authenticate_authorization_value_str(&auth_header)
            .expect("initial call should warm cache");
        let before = request_count.load(Ordering::SeqCst);

        std::thread::sleep(Duration::from_millis(1_100));

        let mut workers = Vec::new();
        for _ in 0..8 {
            let cfg = Arc::clone(&cfg);
            let auth_header = auth_header.clone();
            workers.push(thread::spawn(move || {
                cfg.authenticate_authorization_value_str(&auth_header)
                    .expect("concurrent auth call should succeed");
            }));
        }

        for worker in workers {
            worker.join().expect("worker thread should join");
        }

        let after = request_count.load(Ordering::SeqCst);
        assert_eq!(
            after,
            before + 1,
            "only one refresh fetch should occur per refresh interval under concurrency"
        );

        let _ = shutdown_tx.send(());
    }

    #[test]
    fn jwks_unreachable_endpoint_returns_misconfigured() {
        // Port 9 is traditionally discard service and is expected to be closed in local tests.
        let jwks_url = "http://127.0.0.1:9/jwks".to_string();
        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 300,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                300,
                default_jwks_algorithms(),
            ))),
        };

        let token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        let err = cfg
            .authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect_err("unreachable jwks endpoint should fail");

        match err {
            AuthRejection::Misconfigured(message) => {
                assert!(message.contains("failed to fetch jwks"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn jwks_malformed_payload_without_cache_returns_misconfigured() {
        let (jwks_url, _payload, _request_count, shutdown_tx) =
            spawn_jwks_server("{ invalid-json".to_string());
        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 300,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                300,
                default_jwks_algorithms(),
            ))),
        };

        let token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        let err = cfg
            .authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect_err("malformed jwks payload should fail");

        match err {
            AuthRejection::Misconfigured(message) => {
                assert!(message.contains("invalid jwks payload"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = shutdown_tx.send(());
    }

    #[test]
    fn jwks_rejects_oversized_payload() {
        let oversized = "a".repeat(MAX_JWKS_RESPONSE_BYTES + 16);
        let (jwks_url, _payload, _request_count, shutdown_tx) = spawn_jwks_server(oversized);
        let cfg = AuthRuntimeConfig {
            enabled: true,
            jwt_secret: None,
            jwks_url: Some(jwks_url.clone()),
            jwks_refresh_secs: 300,
            jwks_allowed_algorithms: default_jwks_algorithms(),
            expected_issuer: Some("https://issuer.local".to_string()),
            expected_audience: Some("openportio-api".to_string()),
            jwks_provider: Some(Arc::new(JwksProvider::new(
                jwks_url,
                300,
                default_jwks_algorithms(),
            ))),
        };

        let token = build_rs256_token("rsa-key-1", "https://issuer.local", "openportio-api");
        let err = cfg
            .authenticate_authorization_value_str(&format!("Bearer {token}"))
            .expect_err("oversized jwks payload should fail");

        match err {
            AuthRejection::Misconfigured(message) => {
                assert!(
                    message.contains("exceeds max size")
                        || message.contains("failed to read jwks body"),
                    "expected oversized-body rejection, got: {message}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = shutdown_tx.send(());
    }

    fn build_rs256_token(kid: &str, issuer: &str, audience: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let claims = JwtClaims {
            sub: "user-1".to_string(),
            exp: 4_102_444_800,
            iss: Some(issuer.to_string()),
            aud: Some(AudienceClaim::One(audience.to_string())),
            scope: Some("read:notes".to_string()),
        };
        let encoding_key = EncodingKey::from_rsa_der(TEST_RSA_PRIVATE_KEY_DER);
        encode(&header, &claims, &encoding_key).expect("token should encode")
    }

    fn build_jwks_json(kid: &str) -> String {
        let encoding_key = EncodingKey::from_rsa_der(TEST_RSA_PRIVATE_KEY_DER);
        let mut jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::RS256)
            .expect("jwk should be generated");
        jwk.common.key_id = Some(kid.to_string());
        serde_json::to_string(&JwkSet { keys: vec![jwk] }).expect("jwks should serialize")
    }

    fn spawn_jwks_server(
        initial_payload: String,
    ) -> (
        String,
        Arc<Mutex<String>>,
        Arc<AtomicUsize>,
        mpsc::Sender<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        listener
            .set_nonblocking(true)
            .expect("listener should be nonblocking");
        let addr = listener.local_addr().expect("addr should resolve");

        let payload = Arc::new(Mutex::new(initial_payload));
        let payload_for_server = Arc::clone(&payload);
        let request_count = Arc::new(AtomicUsize::new(0));
        let request_count_for_server = Arc::clone(&request_count);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        thread::spawn(move || loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }

            match listener.accept() {
                Ok((mut stream, _)) => {
                    request_count_for_server.fetch_add(1, Ordering::SeqCst);
                    let mut request_buffer = [0_u8; 1024];
                    let _ = stream.read(&mut request_buffer);

                    let body = payload_for_server.lock().expect("payload lock").clone();
                    let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        });

        (
            format!("http://{addr}/jwks"),
            payload,
            request_count,
            shutdown_tx,
        )
    }

    fn clear_auth_env() {
        for key in [
            "OPENPORTIO_AUTH_ENABLED",
            "OPENPORTIO_AUTH_JWT_SECRET",
            "OPENPORTIO_AUTH_JWKS_URL",
            "OPENPORTIO_AUTH_JWKS_REFRESH_SECS",
            "OPENPORTIO_AUTH_JWKS_ALGORITHMS",
            "OPENPORTIO_AUTH_ISSUER",
            "OPENPORTIO_AUTH_AUDIENCE",
            "MELD_AUTH_ENABLED",
            "MELD_AUTH_JWT_SECRET",
            "MELD_AUTH_JWKS_URL",
            "MELD_AUTH_JWKS_REFRESH_SECS",
            "MELD_AUTH_JWKS_ALGORITHMS",
            "MELD_AUTH_ISSUER",
            "MELD_AUTH_AUDIENCE",
            "ALLOY_AUTH_ENABLED",
            "ALLOY_AUTH_JWT_SECRET",
            "ALLOY_AUTH_JWKS_URL",
            "ALLOY_AUTH_JWKS_REFRESH_SECS",
            "ALLOY_AUTH_JWKS_ALGORITHMS",
            "ALLOY_AUTH_ISSUER",
            "ALLOY_AUTH_AUDIENCE",
        ] {
            env::remove_var(key);
        }
    }
}
