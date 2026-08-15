CREATE TABLE review_history (
    id INTEGER PRIMARY KEY,
    question_id INTEGER NOT NULL,
    is_correct INTEGER NOT NULL CHECK (is_correct IN (0, 1)),
    reviewed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (question_id) REFERENCES questions(id) ON DELETE CASCADE
);

CREATE INDEX idx_review_history_reviewed_at
ON review_history(reviewed_at);

CREATE INDEX idx_review_history_question_id
ON review_history(question_id);
