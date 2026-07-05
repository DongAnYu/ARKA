CREATE TABLE IF NOT EXISTS model_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    provider TEXT NOT NULL DEFAULT 'ollama' CHECK (provider IN ('ollama', 'openrouter')),
    base_url TEXT NOT NULL DEFAULT 'http://localhost:11434',
    selected_model TEXT NOT NULL DEFAULT '',
    timeout_secs INTEGER NOT NULL DEFAULT 60,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT OR IGNORE INTO model_settings (id, provider, base_url, selected_model, timeout_secs)
VALUES (1, 'ollama', 'http://localhost:11434', '', 60);
