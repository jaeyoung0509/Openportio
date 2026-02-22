CREATE INDEX IF NOT EXISTS notes_search_fts_idx
ON notes
USING GIN (to_tsvector('simple', COALESCE(title, '') || ' ' || COALESCE(body, '')));
