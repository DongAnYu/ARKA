-- SQLx owns versioning, checksums and the transaction for this migration.
-- This migration upgrades the released questions/review_history schema.
CREATE TEMP TABLE _learning_item_guard (
    ok INTEGER NOT NULL CONSTRAINT learning_item_migration_validation CHECK(ok=1)
);
INSERT INTO _learning_item_guard SELECT CASE WHEN
    (EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='questions')
      AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='review_history')
      AND NOT EXISTS(SELECT 1 FROM sqlite_master WHERE name='learning_items'))
    THEN 1 ELSE 0 END;
-- Never infer a correct answer from an invalid label.
INSERT INTO _learning_item_guard SELECT CASE WHEN
    EXISTS(SELECT 1 FROM questions WHERE correct_answer NOT IN ('A','B','C','D'))
    THEN 0 ELSE 1 END;
CREATE TABLE IF NOT EXISTS learning_items (
    id INTEGER PRIMARY KEY,
    target TEXT,
    answer TEXT,
    explanation TEXT,
    space_id INTEGER NOT NULL REFERENCES recall_spaces(id),
    model TEXT,
    provider TEXT,
    pipeline TEXT CHECK (pipeline IN ('chunk', 'graph')),
    generated_at TEXT,
    source_json TEXT CHECK (source_json IS NULL OR json_valid(source_json)),
    status TEXT NOT NULL DEFAULT 'ready' CHECK (status IN ('ready', 'needs_repair')),
    repetitions INTEGER NOT NULL DEFAULT 0,
    interval_days INTEGER NOT NULL DEFAULT 0,
    ease_factor REAL NOT NULL DEFAULT 2.5,
    next_review_at TEXT,
    last_reviewed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_learning_items_space_due ON learning_items(space_id, next_review_at);
CREATE INDEX IF NOT EXISTS idx_learning_items_due ON learning_items(next_review_at);

CREATE TABLE IF NOT EXISTS question_variants (
    id INTEGER PRIMARY KEY,
    learning_item_id INTEGER NOT NULL REFERENCES learning_items(id) ON DELETE CASCADE,
    format TEXT NOT NULL CHECK (format IN ('mcq', 'flashcard')),
    prompt TEXT NOT NULL,
    content_json TEXT NOT NULL CHECK (json_valid(content_json)),
    UNIQUE(learning_item_id, format)
);
CREATE TABLE IF NOT EXISTS legacy_review_history (
    id INTEGER PRIMARY KEY,
    question_id INTEGER NOT NULL REFERENCES learning_items(id) ON DELETE CASCADE,
    is_correct INTEGER NOT NULL CHECK (is_correct IN (0, 1)),
    reviewed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TEMP TABLE _learning_item_before AS SELECT
    (SELECT COUNT(*) FROM learning_items) AS items,
    (SELECT COUNT(*) FROM question_variants) AS variants,
    (SELECT COUNT(*) FROM legacy_review_history) AS events;
INSERT INTO learning_items (
    id, answer, explanation, space_id, model,
    repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at
)
SELECT id, CASE correct_answer
    WHEN 'A' THEN option_a WHEN 'B' THEN option_b
    WHEN 'C' THEN option_c WHEN 'D' THEN option_d END,
    explanation, space_id, model,
    repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at
FROM questions;

INSERT INTO question_variants (id, learning_item_id, format, prompt, content_json)
SELECT id, id, 'mcq', question, json_object(
    'options', json_array(
        json_object('id', 'A', 'text', option_a),
        json_object('id', 'B', 'text', option_b),
        json_object('id', 'C', 'text', option_c),
        json_object('id', 'D', 'text', option_d)),
    'correct_option_id', correct_answer)
FROM questions;

INSERT INTO legacy_review_history (id, question_id, is_correct, reviewed_at)
SELECT id, question_id, is_correct, reviewed_at
FROM review_history;
CREATE INDEX IF NOT EXISTS idx_legacy_review_history_question ON legacy_review_history(question_id);


INSERT INTO _learning_item_guard SELECT CASE WHEN (
        SELECT
          (SELECT COUNT(*) FROM questions)+(SELECT items FROM _learning_item_before) != (SELECT COUNT(*) FROM learning_items)
          OR (SELECT COUNT(*) FROM questions)+(SELECT variants FROM _learning_item_before) != (SELECT COUNT(*) FROM question_variants)
          OR (SELECT COUNT(*) FROM review_history)+(SELECT events FROM _learning_item_before) != (SELECT COUNT(*) FROM legacy_review_history)
          OR EXISTS (
            SELECT 1 FROM questions q LEFT JOIN learning_items i ON i.id=q.id
            LEFT JOIN question_variants v ON v.id=q.id AND v.learning_item_id=q.id
            WHERE i.id IS NULL OR v.id IS NULL
              OR i.model IS NOT q.model OR i.space_id IS NOT q.space_id
              OR i.explanation IS NOT q.explanation OR i.target IS NOT NULL
              OR i.repetitions IS NOT q.repetitions OR i.interval_days IS NOT q.interval_days
              OR i.ease_factor IS NOT q.ease_factor OR i.next_review_at IS NOT q.next_review_at
              OR i.last_reviewed_at IS NOT q.last_reviewed_at
              OR i.answer IS NOT CASE q.correct_answer WHEN 'A' THEN q.option_a WHEN 'B' THEN q.option_b WHEN 'C' THEN q.option_c WHEN 'D' THEN q.option_d END
              OR v.format != 'mcq' OR v.prompt IS NOT q.question
              OR json_extract(v.content_json, '$.options[0].text') IS NOT q.option_a
              OR json_extract(v.content_json, '$.options[1].text') IS NOT q.option_b
              OR json_extract(v.content_json, '$.options[2].text') IS NOT q.option_c
              OR json_extract(v.content_json, '$.options[3].text') IS NOT q.option_d
              OR json_extract(v.content_json, '$.correct_option_id') IS NOT q.correct_answer)
          OR EXISTS (
            SELECT 1 FROM review_history h LEFT JOIN legacy_review_history n ON n.id=h.id
            WHERE n.id IS NULL OR n.question_id IS NOT h.question_id
              OR n.is_correct IS NOT h.is_correct OR n.reviewed_at IS NOT h.reviewed_at)
    ) THEN 0 ELSE 1 END;

DROP TABLE review_history;
DROP TABLE questions;
INSERT INTO _learning_item_guard SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_foreign_key_check) THEN 0 ELSE 1 END;
DROP TABLE _learning_item_before;
DROP TABLE _learning_item_guard;
