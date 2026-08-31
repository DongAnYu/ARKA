CREATE TABLE model_settings_with_openai (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    provider TEXT NOT NULL DEFAULT 'ollama'
        CHECK (provider IN ('ollama', 'openai', 'openrouter')),
    base_url TEXT NOT NULL DEFAULT 'http://localhost:11434',
    selected_model TEXT NOT NULL DEFAULT '',
    timeout_secs INTEGER NOT NULL DEFAULT 60,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    api_key TEXT
);

INSERT INTO model_settings_with_openai (
    id,
    provider,
    base_url,
    selected_model,
    timeout_secs,
    updated_at,
    api_key
)
SELECT
    id,
    provider,
    base_url,
    selected_model,
    timeout_secs,
    updated_at,
    api_key
FROM model_settings;

DROP TABLE model_settings;

ALTER TABLE model_settings_with_openai RENAME TO model_settings;
