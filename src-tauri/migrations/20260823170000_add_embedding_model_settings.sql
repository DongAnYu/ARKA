ALTER TABLE model_settings
ADD COLUMN embedding_provider TEXT NOT NULL DEFAULT 'ollama'
    CHECK (embedding_provider IN ('ollama', 'openai', 'openrouter'));

ALTER TABLE model_settings
ADD COLUMN embedding_base_url TEXT NOT NULL DEFAULT 'http://localhost:11434';

ALTER TABLE model_settings
ADD COLUMN embedding_selected_model TEXT NOT NULL DEFAULT '';

ALTER TABLE model_settings
ADD COLUMN embedding_timeout_secs INTEGER NOT NULL DEFAULT 60
    CHECK (embedding_timeout_secs > 0);

ALTER TABLE model_settings
ADD COLUMN embedding_api_key TEXT;
