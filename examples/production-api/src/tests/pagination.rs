use chrono::Utc;

use crate::{application::notes::paginate_notes, domain::notes::NoteRow};

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
