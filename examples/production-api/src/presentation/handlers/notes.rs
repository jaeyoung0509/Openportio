use std::sync::Arc;

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    Json,
};
use openportio_core::auth::AuthPrincipal;
use openportio_server::api::{ApiError, ValidatedJson, ValidatedPath, ValidatedQuery};

use crate::{
    application::notes::{normalize_query_filter, paginate_notes},
    domain::notes::{
        NoteResponse, NoteRow, NotesListResponse, NotesPageMeta, ProtectedNoteResponse,
    },
    infrastructure::state::ProductionApiState,
    presentation::{
        dto::{CreateNoteBody, ListNotesQuery, NotePath},
        errors::{database_error, not_found},
    },
};

#[openportio_server::route(post, "/v1/notes", auto_validate, transparent)]
pub(crate) async fn create_note(
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
pub(crate) async fn list_notes(
    Extension(principal): Extension<AuthPrincipal>,
    State(state): State<Arc<ProductionApiState>>,
    ValidatedQuery(query): ValidatedQuery<ListNotesQuery>,
) -> Result<Json<NotesListResponse>, ApiError> {
    let limit = query.limit.unwrap_or(20);
    let limit_plus_one = limit + 1;
    let query_filter = normalize_query_filter(query.q);
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
          AND (
            $2::text IS NULL
            OR to_tsvector('simple', COALESCE(title, '') || ' ' || COALESCE(body, ''))
               @@ websearch_to_tsquery('simple', $2)
          )
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
pub(crate) async fn get_protected_note(
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
