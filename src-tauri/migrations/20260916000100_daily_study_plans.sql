CREATE TABLE study_preferences (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    daily_target INTEGER NOT NULL CHECK (daily_target BETWEEN 1 AND 10000),
    max_new_items INTEGER NOT NULL CHECK (max_new_items BETWEEN 0 AND daily_target)
);
INSERT INTO study_preferences VALUES (1, 20, 5);

CREATE TABLE daily_study_plans (
    id INTEGER PRIMARY KEY,
    local_date TEXT NOT NULL UNIQUE,
    utc_offset_seconds INTEGER NOT NULL,
    daily_target INTEGER NOT NULL,
    max_new_items INTEGER NOT NULL,
    space_id INTEGER REFERENCES recall_spaces(id) ON DELETE SET NULL
);

CREATE TABLE daily_study_plan_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id INTEGER NOT NULL REFERENCES daily_study_plans(id) ON DELETE CASCADE,
    learning_item_id INTEGER REFERENCES learning_items(id) ON DELETE SET NULL,
    was_new INTEGER NOT NULL CHECK (was_new IN (0, 1)),
    is_extra INTEGER NOT NULL DEFAULT 0 CHECK (is_extra IN (0, 1)),
    completed_at TEXT,
    UNIQUE (plan_id, learning_item_id)
);

CREATE TABLE review_events (
    id INTEGER PRIMARY KEY,
    learning_item_id INTEGER REFERENCES learning_items(id) ON DELETE SET NULL,
    plan_item_id INTEGER UNIQUE REFERENCES daily_study_plan_items(id) ON DELETE SET NULL,
    local_date TEXT NOT NULL,
    was_new INTEGER NOT NULL CHECK (was_new IN (0, 1)),
    reviewed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_review_events_day ON review_events(local_date, learning_item_id);
