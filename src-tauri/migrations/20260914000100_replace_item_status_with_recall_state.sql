-- Learning-item content is validated before persistence, so the old
-- ready/needs_repair status is not a reachable production lifecycle.
-- Preserve every existing item as scheduled and reserve new for future saves.
CREATE TEMP TABLE _learning_item_status_guard (
    ok INTEGER NOT NULL CONSTRAINT learning_item_status_validation CHECK(ok=1)
);

INSERT INTO _learning_item_status_guard
SELECT CASE
    WHEN EXISTS(SELECT 1 FROM learning_items WHERE status != 'ready') THEN 0
    ELSE 1
END;

ALTER TABLE learning_items
ADD COLUMN recall_state TEXT NOT NULL DEFAULT 'scheduled'
CHECK (recall_state IN ('new', 'scheduled'));

ALTER TABLE learning_items DROP COLUMN status;

CREATE INDEX idx_learning_items_recall_state_due
ON learning_items(recall_state, next_review_at, space_id);

DROP TABLE _learning_item_status_guard;
