-- Data only. The schema comes from the actual migrations released through v0.1.6.
-- These rows exercise every answer label, NULL and empty metadata, Unicode,
-- multiline content, recall-space membership, and varied SM-2 schedules.
INSERT INTO recall_spaces (id, name, description)
VALUES (7, 'Algorithms 算法', 'Preserve Unicode');

INSERT INTO questions (
    id, question, option_a, option_b, option_c, option_d, correct_answer,
    explanation, model, space_id, repetitions, interval_days, ease_factor,
    next_review_at, last_reviewed_at
)
VALUES
    (10, 'Which statement is true?', 'first', 'second', 'third', 'fourth', 'A', NULL, 'old-model', 1, 0, 0, 2.5, NULL, NULL),
    (20, 'Binary search worst-case time?', 'O(1)', 'O(log n)', 'O(n)', 'O(n²)', 'B', 'Halves the interval.', '', 7, 4, 38, 2.5, '2026-10-10 12:00:00', '2026-09-02 12:00:00'),
    (30, '含义是什么？', '甲', '乙', '丙', '丁', 'C', '解释\n保留', NULL, 7, 2, 6, 1.3, '2020-01-01 00:00:00', '2019-12-26 00:00:00'),
    (40, 'Quoted "prompt"', 'a', 'b', 'c', 'd', 'D', 'line one
line two', 'provider/model:tag', 1, 8, 120, 2.66, '2030-01-01 00:00:00', '2029-09-03 00:00:00');

-- Preserve both correctness outcomes and non-sequential historical IDs.
INSERT INTO review_history (id, question_id, is_correct, reviewed_at)
VALUES
    (101, 20, 1, '2026-09-02 12:00:00'),
    (105, 20, 0, '2026-08-01 01:02:03'),
    (110, 30, 1, '2019-12-26 00:00:00');

-- This is an explicit fake key used to prove model settings survive migration.
UPDATE model_settings
SET selected_model = 'configured-model',
    api_key = 'fixture-only-not-a-real-key',
    llm_concurrency = 3
WHERE id = 1;
