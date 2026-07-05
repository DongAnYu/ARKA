CREATE TABLE model_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    provider TEXT NOT NULL DEFAULT 'ollama' CHECK (provider IN ('ollama', 'openrouter')),
    base_url TEXT NOT NULL DEFAULT 'http://localhost:11434',
    selected_model TEXT NOT NULL DEFAULT '',
    api_key TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT OR IGNORE INTO model_settings (id, provider, base_url, selected_model, api_key)
VALUES (1, 'ollama', 'http://localhost:11434', '', NULL);
