-- Add migration script here
ALTER TABLE questions
ADD COLUMN repetitions INTEGER NOT NULL DEFAULT 0;

ALTER TABLE questions
ADD COLUMN interval_days INTEGER NOT NULL DEFAULT 0;

ALTER TABLE questions
ADD COLUMN ease_factor REAL NOT NULL DEFAULT 2.5;

ALTER TABLE questions
ADD COLUMN next_review_at TEXT;

ALTER TABLE questions
ADD COLUMN last_reviewed_at TEXT;