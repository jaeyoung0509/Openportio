use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Serialize)]
pub(crate) struct NoteResponse {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct NotesPageMeta {
    pub(crate) limit: i64,
    pub(crate) cursor: Option<i64>,
    pub(crate) next_cursor: Option<i64>,
    pub(crate) has_more: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct NotesListResponse {
    pub(crate) notes: Vec<NoteResponse>,
    pub(crate) page: NotesPageMeta,
    pub(crate) query: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProtectedNoteResponse {
    pub(crate) note: NoteResponse,
    pub(crate) subject: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct NoteRow {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
}

impl NoteRow {
    pub(crate) fn into_response(self) -> NoteResponse {
        NoteResponse {
            id: self.id,
            title: self.title,
            body: self.body,
            created_at: self.created_at,
        }
    }
}
