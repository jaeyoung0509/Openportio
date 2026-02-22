use crate::domain::notes::NoteRow;

pub(crate) fn normalize_query_filter(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn paginate_notes(
    mut rows: Vec<NoteRow>,
    limit: i64,
) -> (Vec<NoteRow>, bool, Option<i64>) {
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
