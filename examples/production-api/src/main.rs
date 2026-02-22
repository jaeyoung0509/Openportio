use std::{env, error::Error, fmt, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    middleware::from_fn_with_state,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use openportio_core::{auth::AuthPrincipal, AppState};
use openportio_server::{
    api::{ApiError, ApiErrorResponse, ValidatedJson, ValidatedPath, ValidatedQuery},
    auth::{self, AuthRuntimeConfig},
    OpenportioServer,
};
use serde::Serialize;
use sqlx::{postgres::PgPoolOptions, FromRow, PgPool};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone)]
struct ProductionConfig {
    addr: SocketAddr,
    service_name: String,
    database_url: String,
    max_db_connections: u32,
    run_migrations: bool,
    migration_retry_seconds: u64,
    enable_drill_routes: bool,
}

#[derive(Debug)]
enum ConfigError {
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
    fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    fn from_lookup<F>(mut lookup: F) -> Result<Self, ConfigError>
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

#[derive(Clone)]
struct ProductionApiState {
    service_name: String,
    pool: PgPool,
}

impl ProductionApiState {
    fn new(service_name: String, pool: PgPool) -> Self {
        Self { service_name, pool }
    }
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: String,
}

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    database: &'static str,
}

#[derive(Debug, Serialize)]
struct NoteResponse {
    id: i64,
    title: String,
    body: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct NotesPageMeta {
    limit: i64,
    cursor: Option<i64>,
    next_cursor: Option<i64>,
    has_more: bool,
}

#[derive(Debug, Serialize)]
struct NotesListResponse {
    notes: Vec<NoteResponse>,
    page: NotesPageMeta,
    query: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProtectedNoteResponse {
    note: NoteResponse,
    subject: String,
}

#[derive(Debug, FromRow)]
struct NoteRow {
    id: i64,
    title: String,
    body: Option<String>,
    created_at: DateTime<Utc>,
}

#[openportio_server::dto]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
    #[validate(length(max = 2000))]
    body: Option<String>,
}

#[openportio_server::dto]
struct ListNotesQuery {
    #[validate(range(min = 1, max = 100))]
    limit: Option<i64>,
    #[validate(length(max = 80))]
    q: Option<String>,
    #[validate(range(min = 1))]
    cursor: Option<i64>,
}

#[openportio_server::dto]
struct NotePath {
    #[validate(range(min = 1))]
    id: i64,
}

#[openportio_server::dto]
struct DrillSleepPath {
    #[validate(range(min = 1, max = 30))]
    seconds: u64,
}

#[openportio_server::route(get, "/livez")]
async fn livez() -> Json<StatusResponse> {
    Json(StatusResponse { status: "live" })
}

#[openportio_server::route(get, "/health")]
async fn health(State(state): State<Arc<ProductionApiState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: state.service_name.clone(),
    })
}

#[openportio_server::route(get, "/readyz")]
async fn readyz(
    State(state): State<Arc<ProductionApiState>>,
) -> Result<Json<ReadyResponse>, ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .map_err(readiness_error)?;

    Ok(Json(ReadyResponse {
        status: "ready",
        database: "ok",
    }))
}

#[openportio_server::route(post, "/v1/notes", auto_validate, transparent)]
async fn create_note(
    Extension(principal): Extension<AuthPrincipal>,
    State(state): State<Arc<ProductionApiState>>,
    ValidatedJson(body): ValidatedJson<CreateNoteBody>,
) -> Result<(StatusCode, Json<NoteResponse>), ApiError> {
    let note = sqlx::query_as::<_, NoteRow>(
        r#"
        INSERT INTO notes (owner_subject, title, body)
        VALUES ($1, $2, $3)
        RETURNING
            id,
            title,
            body,
            created_at
        "#,
    )
    .bind(principal.subject)
    .bind(body.title)
    .bind(body.body)
    .fetch_one(&state.pool)
    .await
    .map_err(database_error)?;

    Ok((StatusCode::CREATED, Json(note.into_response())))
}

#[openportio_server::route(get, "/v1/notes", auto_validate, transparent)]
async fn list_notes(
    Extension(principal): Extension<AuthPrincipal>,
    State(state): State<Arc<ProductionApiState>>,
    ValidatedQuery(query): ValidatedQuery<ListNotesQuery>,
) -> Result<Json<NotesListResponse>, ApiError> {
    let limit = query.limit.unwrap_or(20);
    let limit_plus_one = limit + 1;
    let query_filter = query.q.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let cursor = query.cursor;

    let rows = sqlx::query_as::<_, NoteRow>(
        r#"
        SELECT
            id,
            title,
            body,
            created_at
        FROM notes
        WHERE owner_subject = $1
          AND ($2::text IS NULL OR title ILIKE ('%' || $2 || '%') OR COALESCE(body, '') ILIKE ('%' || $2 || '%'))
          AND ($3::bigint IS NULL OR id < $3)
        ORDER BY id DESC
        LIMIT $4
        "#,
    )
    .bind(principal.subject)
    .bind(query_filter.as_deref())
    .bind(cursor)
    .bind(limit_plus_one)
    .fetch_all(&state.pool)
    .await
    .map_err(database_error)?;

    let (notes, has_more, next_cursor) = paginate_notes(rows, limit);
    Ok(Json(NotesListResponse {
        notes: notes
            .into_iter()
            .map(NoteRow::into_response)
            .collect::<Vec<_>>(),
        page: NotesPageMeta {
            limit,
            cursor,
            next_cursor,
            has_more,
        },
        query: query_filter,
    }))
}

#[openportio_server::route(get, "/protected/notes/:id", auto_validate, transparent)]
async fn get_protected_note(
    Extension(principal): Extension<AuthPrincipal>,
    State(state): State<Arc<ProductionApiState>>,
    ValidatedPath(path): ValidatedPath<NotePath>,
) -> Result<Json<ProtectedNoteResponse>, ApiError> {
    let subject = principal.subject;
    let maybe_note = sqlx::query_as::<_, NoteRow>(
        r#"
        SELECT
            id,
            title,
            body,
            created_at
        FROM notes
        WHERE id = $1 AND owner_subject = $2
        "#,
    )
    .bind(path.id)
    .bind(&subject)
    .fetch_optional(&state.pool)
    .await
    .map_err(database_error)?;

    let note = maybe_note.ok_or_else(|| not_found(format!("note {} was not found", path.id)))?;

    Ok(Json(ProtectedNoteResponse {
        note: note.into_response(),
        subject,
    }))
}

#[openportio_server::route(get, "/ops/drill/sleep/:seconds", auto_validate, transparent)]
async fn drill_sleep(ValidatedPath(path): ValidatedPath<DrillSleepPath>) -> Json<StatusResponse> {
    tokio::time::sleep(Duration::from_secs(path.seconds)).await;
    Json(StatusResponse {
        status: "completed",
    })
}

impl NoteRow {
    fn into_response(self) -> NoteResponse {
        NoteResponse {
            id: self.id,
            title: self.title,
            body: self.body,
            created_at: self.created_at,
        }
    }
}

fn paginate_notes(mut rows: Vec<NoteRow>, limit: i64) -> (Vec<NoteRow>, bool, Option<i64>) {
    let has_more = (rows.len() as i64) > limit;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        rows.last().map(|row| row.id)
    } else {
        None
    };
    (rows, has_more, next_cursor)
}

fn not_found(message: String) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResponse {
            code: "not_found".to_string(),
            message,
            detail: None,
            details: None,
        }),
    )
}

fn readiness_error(err: sqlx::Error) -> ApiError {
    tracing::warn!(error = %err, "readiness check failed");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ApiErrorResponse {
            code: "not_ready".to_string(),
            message: "database is unavailable".to_string(),
            detail: None,
            details: None,
        }),
    )
}

fn database_error(err: sqlx::Error) -> ApiError {
    tracing::error!(error = %err, "database operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse::internal_server_error()),
    )
}

fn build_rest_router(
    state: Arc<ProductionApiState>,
    auth_cfg: AuthRuntimeConfig,
    enable_drill_routes: bool,
) -> Router {
    let mut protected_router = Router::new()
        .route("/v1/notes", get(list_notes).post(create_note))
        .route("/protected/notes/:id", get(get_protected_note));
    if enable_drill_routes {
        protected_router = protected_router.route("/ops/drill/sleep/:seconds", get(drill_sleep));
    }

    let notes_router =
        protected_router.route_layer(from_fn_with_state(auth_cfg, auth::rest_auth_middleware));

    Router::new()
        .route("/livez", get(livez))
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .merge(notes_router)
        .with_state(state)
}

fn spawn_migration_worker(pool: PgPool, retry_seconds: u64) {
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

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    init_tracing();
    let config = ProductionConfig::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(config.max_db_connections)
        .connect_lazy(&config.database_url)?;

    if config.run_migrations {
        spawn_migration_worker(pool.clone(), config.migration_retry_seconds);
    }

    let rest_state = Arc::new(ProductionApiState::new(config.service_name.clone(), pool));
    let rest_router = build_rest_router(
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, body::Body, http::Request};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use openportio_core::auth::{AudienceClaim, JwtClaims};
    use openportio_server::middleware::MiddlewareConfig;
    use tower::util::ServiceExt;

    fn auth_cfg_for_tests() -> AuthRuntimeConfig {
        let mut auth_cfg = AuthRuntimeConfig::default();
        auth_cfg.enabled = true;
        auth_cfg.jwt_secret = Some("dev-secret".to_string());
        auth_cfg.expected_issuer = Some("https://issuer.local".to_string());
        auth_cfg.expected_audience = Some("openportio-api".to_string());
        auth_cfg
    }

    fn issue_test_token(secret: &str, subject: &str) -> String {
        let claims = JwtClaims {
            sub: subject.to_string(),
            exp: 4_102_444_800,
            iss: Some("https://issuer.local".to_string()),
            aud: Some(AudienceClaim::One("openportio-api".to_string())),
            scope: Some("read:notes write:notes".to_string()),
        };

        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token should encode")
    }

    #[test]
    fn config_requires_database_url() {
        let cfg = ProductionConfig::from_lookup(|key| match key {
            "PROD_API_ADDR" => Some("127.0.0.1:4100".to_string()),
            _ => None,
        });
        assert!(matches!(
            cfg,
            Err(ConfigError::MissingRequired {
                key: "PROD_API_DATABASE_URL"
            })
        ));
    }

    #[test]
    fn config_uses_defaults_when_optional_values_absent() {
        let cfg = ProductionConfig::from_lookup(|key| {
            if key == "PROD_API_DATABASE_URL" {
                Some("postgres://127.0.0.1:55432/openportio".to_string())
            } else {
                None
            }
        })
        .expect("config should parse");

        assert_eq!(cfg.addr, SocketAddr::from(([127, 0, 0, 1], 4100)));
        assert_eq!(cfg.service_name, "production-api");
        assert_eq!(cfg.max_db_connections, 10);
        assert!(cfg.run_migrations);
        assert_eq!(cfg.migration_retry_seconds, 5);
        assert!(!cfg.enable_drill_routes);
    }

    #[tokio::test]
    async fn readyz_returns_503_when_database_is_unavailable() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");

        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let app = build_rest_router(state, AuthRuntimeConfig::default(), false);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn notes_routes_require_auth_when_enabled() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");

        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let app = build_rest_router(state, auth_cfg_for_tests(), false);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/notes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ApiErrorResponse =
            serde_json::from_slice(&body).expect("error body should parse");
        assert_eq!(parsed.code, "unauthorized");
    }

    #[tokio::test]
    async fn notes_route_rejects_invalid_bearer_token() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");
        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let app = build_rest_router(state, auth_cfg_for_tests(), false);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/notes")
                    .header("authorization", "Bearer invalid-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ApiErrorResponse =
            serde_json::from_slice(&body).expect("error body should parse");
        assert_eq!(parsed.code, "unauthorized");
    }

    #[tokio::test]
    async fn notes_route_returns_500_when_database_is_unavailable() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");
        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let app = build_rest_router(state, auth_cfg_for_tests(), false);
        let token = issue_test_token("dev-secret", "test-user");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/notes?limit=5")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ApiErrorResponse =
            serde_json::from_slice(&body).expect("error body should parse");
        assert_eq!(parsed.code, "internal_error");
    }

    #[tokio::test]
    async fn notes_route_validation_failure_returns_400_before_database_access() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");
        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let app = build_rest_router(state, auth_cfg_for_tests(), false);
        let token = issue_test_token("dev-secret", "test-user");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/notes?limit=101")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let parsed: ApiErrorResponse =
            serde_json::from_slice(&body).expect("error body should parse");
        assert_eq!(parsed.code, "validation_error");
    }

    #[test]
    fn paginate_notes_returns_next_cursor_only_when_more_results_exist() {
        let make_note = |id: i64| NoteRow {
            id,
            title: format!("note-{id}"),
            body: None,
            created_at: Utc::now(),
        };

        let rows = vec![make_note(10), make_note(9), make_note(8), make_note(7)];
        let (page, has_more, next_cursor) = paginate_notes(rows, 3);
        assert_eq!(page.len(), 3);
        assert!(has_more);
        assert_eq!(next_cursor, Some(8));

        let rows = vec![make_note(3), make_note(2)];
        let (page, has_more, next_cursor) = paginate_notes(rows, 3);
        assert_eq!(page.len(), 2);
        assert!(!has_more);
        assert_eq!(next_cursor, None);
    }

    #[tokio::test]
    async fn livez_stays_ok_when_readyz_is_degraded() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");
        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let app = build_rest_router(state, AuthRuntimeConfig::default(), false);

        let readyz = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("readyz request should complete");
        assert_eq!(readyz.status(), StatusCode::SERVICE_UNAVAILABLE);

        let livez = app
            .oneshot(
                Request::builder()
                    .uri("/livez")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("livez request should complete");
        assert_eq!(livez.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn drill_route_times_out_when_timeout_budget_is_exceeded() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://127.0.0.1:1/openportio")
            .expect("lazy pool should build");
        let state = Arc::new(ProductionApiState::new(
            "test-production-api".to_string(),
            pool,
        ));
        let rest_router = build_rest_router(state, auth_cfg_for_tests(), true);
        let app = OpenportioServer::new()
            .without_grpc()
            .with_rest_router(rest_router)
            .with_middleware_config(MiddlewareConfig {
                timeout_seconds: 1,
                ..MiddlewareConfig::default()
            })
            .build_app();

        let token = issue_test_token("dev-secret", "test-user");
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ops/drill/sleep/2")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let message = String::from_utf8(body.to_vec()).expect("timeout body should be utf8");
        assert!(message.contains("request timed out"));
    }
}
