-- Replace persisted assignments with a small daily settings snapshot and a
-- durable event ledger. Existing preferences, daily limits, and completed
-- reviews are copied before the old tables are removed.
CREATE TABLE daily_study_state (
    local_date TEXT PRIMARY KEY,
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    utc_offset_seconds INTEGER NOT NULL,
    daily_target INTEGER NOT NULL CHECK (daily_target BETWEEN 1 AND 10000),
    max_new_items INTEGER NOT NULL CHECK (max_new_items BETWEEN 0 AND daily_target)
);

INSERT INTO daily_study_state (
    local_date,
    started_at,
    utc_offset_seconds,
    daily_target,
    max_new_items
)
SELECT
    plan.local_date,
    CURRENT_TIMESTAMP,
    plan.utc_offset_seconds,
    plan.daily_target,
    plan.max_new_items
FROM daily_study_plans plan;

CREATE TABLE review_events_next (
    id INTEGER PRIMARY KEY,
    learning_item_id INTEGER REFERENCES learning_items(id) ON DELETE SET NULL,
    local_date TEXT NOT NULL,
    was_new INTEGER NOT NULL CHECK (was_new IN (0, 1)),
    is_extra INTEGER NOT NULL DEFAULT 0 CHECK (is_extra IN (0, 1)),
    reviewed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO review_events_next (
    id,
    learning_item_id,
    local_date,
    was_new,
    is_extra,
    reviewed_at
)
SELECT
    event.id,
    event.learning_item_id,
    event.local_date,
    event.was_new,
    COALESCE(plan_item.is_extra, 0),
    event.reviewed_at
FROM review_events event
LEFT JOIN daily_study_plan_items plan_item ON plan_item.id = event.plan_item_id;

CREATE TEMP TABLE _daily_study_migration_guard (
    ok INTEGER NOT NULL CONSTRAINT daily_study_migration_validation CHECK (ok = 1)
);

INSERT INTO _daily_study_migration_guard
SELECT CASE WHEN
    (SELECT COUNT(*) FROM daily_study_plans) = (SELECT COUNT(*) FROM daily_study_state)
    AND (SELECT COUNT(*) FROM review_events) = (SELECT COUNT(*) FROM review_events_next)
    AND NOT EXISTS (
        SELECT 1
        FROM review_events old
        LEFT JOIN review_events_next new ON new.id = old.id
        WHERE new.id IS NULL
           OR new.learning_item_id IS NOT old.learning_item_id
           OR new.local_date IS NOT old.local_date
           OR new.was_new IS NOT old.was_new
           OR new.reviewed_at IS NOT old.reviewed_at
    )
THEN 1 ELSE 0 END;

DROP TABLE review_events;
DROP TABLE daily_study_plan_items;
DROP TABLE daily_study_plans;
ALTER TABLE review_events_next RENAME TO review_events;

CREATE INDEX idx_review_events_day
ON review_events(local_date, is_extra, was_new);

INSERT INTO _daily_study_migration_guard
SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_foreign_key_check) THEN 0 ELSE 1 END;

DROP TABLE _daily_study_migration_guard;
