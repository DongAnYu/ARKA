ALTER TABLE model_settings
ADD COLUMN llm_concurrency INTEGER NOT NULL DEFAULT 5
    CHECK (llm_concurrency BETWEEN 1 AND 20);
