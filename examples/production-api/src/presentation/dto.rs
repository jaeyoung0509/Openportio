#[openportio_server::dto]
pub(crate) struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    pub(crate) title: String,
    #[validate(length(max = 2000))]
    pub(crate) body: Option<String>,
}

#[openportio_server::dto]
pub(crate) struct ListNotesQuery {
    #[validate(range(min = 1, max = 100))]
    pub(crate) limit: Option<i64>,
    #[validate(length(max = 80))]
    pub(crate) q: Option<String>,
    #[validate(range(min = 1))]
    pub(crate) cursor: Option<i64>,
}

#[openportio_server::dto]
pub(crate) struct NotePath {
    #[validate(range(min = 1))]
    pub(crate) id: i64,
}

#[openportio_server::dto]
pub(crate) struct DrillSleepPath {
    #[validate(range(min = 1, max = 30))]
    pub(crate) seconds: u64,
}

#[openportio_server::dto]
pub(crate) struct GreetingPath {
    #[validate(length(min = 1, max = 120))]
    pub(crate) name: String,
}
