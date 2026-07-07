CREATE TABLE IF NOT EXISTS recall_spaces (
	id INTEGER PRIMARY KEY,
	name TEXT NOT NULL UNIQUE,
	description TEXT
);

INSERT OR IGNORE INTO recall_spaces (id, name, description)
VALUES (1, 'General', 'Default space for ungrouped questions');

CREATE INDEX IF NOT EXISTS idx_questions_space_id
ON questions(space_id);