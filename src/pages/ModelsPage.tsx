import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

type ProviderId = 'ollama' | 'openai' | 'openrouter'
type ConfigScope = 'generation' | 'embedding'
type ModelHistoryMap = Record<string, string[]>
type StatusTone = 'info' | 'success' | 'error'

type ProviderOption = {
  id: ProviderId
  name: string
  description: string
  defaultBaseUrl: string
}

type ProviderConfig = {
  provider: ProviderId
  baseUrl: string
  modelId: string
  timeoutSecs: string
  apiKey: string
}

type PersistedProviderConfig = {
  provider: string
  base_url: string
  selected_model: string
  timeout_secs: number
  api_key: string | null
}

type DbModelConfig = PersistedProviderConfig & {
  llm_concurrency: number
  embedding_provider: string
  embedding_base_url: string
  embedding_selected_model: string
  embedding_timeout_secs: number
  embedding_api_key: string | null
}

type EmbeddingConnectionResult = {
  provider: string
  model: string
  dimensions: number
}

type StatusMessage = {
  message: string
  tone: StatusTone
}

type ModelConfigPanelProps = {
  scope: ConfigScope
  title: string
  description: string
  config: ProviderConfig
  savedConfig: ProviderConfig | null
  modelHistory: ModelHistoryMap
  fetchStatus: StatusMessage | null
  isFetching: boolean
  onChange: (config: ProviderConfig) => void
  onFetchModels: () => void
  onClearStatus: () => void
  llmConcurrency?: string
  onLlmConcurrencyChange?: (value: string) => void
}

const MODEL_HISTORY_KEY = 'models-page-fetch-history-v1'
const DEFAULT_LLM_CONCURRENCY = 5
const MAX_LLM_CONCURRENCY = 20

const providerOptions: ProviderOption[] = [
  {
    id: 'ollama',
    name: 'Ollama',
    description: 'Local models, no API key',
    defaultBaseUrl: 'http://localhost:11434',
  },
  {
    id: 'openai',
    name: 'OpenAI',
    description: 'OpenAI models with an API key',
    defaultBaseUrl: 'https://api.openai.com/v1',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    description: 'Many models, one API key',
    defaultBaseUrl: 'https://openrouter.ai/api/v1',
  },
]

const findProvider = (provider: ProviderId): ProviderOption => {
  return providerOptions.find((option) => option.id === provider) ?? providerOptions[0]
}

const parseProvider = (provider: string): ProviderId => {
  return providerOptions.some((option) => option.id === provider)
    ? (provider as ProviderId)
    : 'ollama'
}

const defaultConfig = (): ProviderConfig => ({
  provider: 'ollama',
  baseUrl: findProvider('ollama').defaultBaseUrl,
  modelId: '',
  timeoutSecs: '60',
  apiKey: '',
})

const formConfig = (config: PersistedProviderConfig): ProviderConfig => ({
  provider: parseProvider(config.provider),
  baseUrl: config.base_url,
  modelId: config.selected_model,
  timeoutSecs: String(config.timeout_secs),
  apiKey: config.api_key ?? '',
})

const embeddingFormConfig = (config: DbModelConfig): ProviderConfig => ({
  provider: parseProvider(config.embedding_provider),
  baseUrl: config.embedding_base_url,
  modelId: config.embedding_selected_model,
  timeoutSecs: String(config.embedding_timeout_secs),
  apiKey: config.embedding_api_key ?? '',
})

const dedupeModels = (models: string[]): string[] => {
  return Array.from(new Set(models.map((item) => item.trim()).filter(Boolean)))
}

const getHistoryKey = (provider: ProviderId, baseUrl: string): string => {
  return `${provider}|${baseUrl.trim().toLowerCase()}`
}

const readModelHistory = (): ModelHistoryMap => {
  try {
    const raw = window.localStorage.getItem(MODEL_HISTORY_KEY)
    if (!raw) {
      return {}
    }

    const parsed = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null) {
      return {}
    }

    return parsed as ModelHistoryMap
  } catch {
    return {}
  }
}

const writeModelHistory = (history: ModelHistoryMap) => {
  window.localStorage.setItem(MODEL_HISTORY_KEY, JSON.stringify(history))
}

const normalizedConfig = (
  config: ProviderConfig,
  label: string,
  allowUnconfigured = false,
): PersistedProviderConfig | StatusMessage => {
  const baseUrl = config.baseUrl.trim()
  const selectedModel = config.modelId.trim()
  const apiKey = config.apiKey.trim()
  const timeoutSecs = Number(config.timeoutSecs)

  if (!baseUrl || (!selectedModel && !allowUnconfigured)) {
    return {
      message: `${label} base URL and model are required before saving.`,
      tone: 'error',
    }
  }

  if (!Number.isInteger(timeoutSecs) || timeoutSecs <= 0) {
    return {
      message: `${label} timeout must be a whole number greater than 0.`,
      tone: 'error',
    }
  }

  if (selectedModel && config.provider !== 'ollama' && !apiKey) {
    return {
      message: `${findProvider(config.provider).name} API key is required for ${label.toLowerCase()}.`,
      tone: 'error',
    }
  }

  return {
    provider: config.provider,
    base_url: baseUrl,
    selected_model: selectedModel,
    timeout_secs: timeoutSecs,
    api_key: config.provider === 'ollama' ? null : apiKey,
  }
}

const isStatusMessage = (
  value: PersistedProviderConfig | StatusMessage,
): value is StatusMessage => 'message' in value

const modelPrompt = (scope: ConfigScope, provider: ProviderId): string => {
  if (scope === 'embedding') {
    return provider === 'openai'
      ? 'e.g. text-embedding-3-small'
      : 'e.g. openai/text-embedding-3-small'
  }

  return provider === 'openai' ? 'e.g. gpt-5.4' : 'e.g. inclusionai/ling-2.6-flash'
}

const statusClassName = (status: StatusMessage): string => {
  return `settings-status${status.tone === 'info' ? '' : ` is-${status.tone}`}`
}

const errorMessage = (error: unknown, fallback: string): string => {
  if (typeof error === 'string' && error.trim()) {
    return error
  }
  if (error instanceof Error && error.message.trim()) {
    return error.message
  }
  return fallback
}

function ModelConfigPanel({
  scope,
  title,
  description,
  config,
  savedConfig,
  modelHistory,
  fetchStatus,
  isFetching,
  onChange,
  onFetchModels,
  onClearStatus,
  llmConcurrency,
  onLlmConcurrencyChange,
}: ModelConfigPanelProps) {
  const activeProvider = useMemo(() => findProvider(config.provider), [config.provider])
  const availableModels = modelHistory[getHistoryKey(config.provider, config.baseUrl)] ?? []
  const prefix = scope === 'generation' ? 'generation' : 'embedding'
  const isEmbedding = scope === 'embedding'

  const update = (changes: Partial<ProviderConfig>) => {
    onChange({ ...config, ...changes })
    onClearStatus()
  }

  const selectProvider = (provider: ProviderId) => {
    if (savedConfig?.provider === provider) {
      onChange(savedConfig)
      onClearStatus()
      return
    }

    update({
      provider,
      baseUrl: findProvider(provider).defaultBaseUrl,
      modelId: '',
      apiKey: '',
    })
  }

  const changeBaseUrl = (baseUrl: string) => {
    const models = modelHistory[getHistoryKey(config.provider, baseUrl)] ?? []
    update({
      baseUrl,
      modelId:
        config.provider === 'ollama' && !models.includes(config.modelId) ? '' : config.modelId,
    })
  }

  return (
    <section className="settings-panel model-config-panel" aria-labelledby={`${prefix}-title`}>
      <header className="model-config-head">
        <div>
          <h2 id={`${prefix}-title`}>{title}</h2>
          <p>{description}</p>
        </div>
        <span className="model-config-provider" aria-label={`Selected provider: ${activeProvider.name}`}>
          {activeProvider.name}
        </span>
      </header>

      <div className="settings-stack">
        <section className="settings-subsection">
          <header className="settings-section-head">
            <h3>Provider</h3>
            <p>Choose where this model runs.</p>
          </header>

          <div className="provider-grid" role="radiogroup" aria-label={`${title} provider`}>
            {providerOptions.map((option) => (
              <button
                key={option.id}
                type="button"
                className={`provider-card${config.provider === option.id ? ' is-active' : ''}`}
                onClick={() => selectProvider(option.id)}
                role="radio"
                aria-checked={config.provider === option.id}
              >
                <span className="provider-card-title">{option.name}</span>
                <span className="provider-card-description">{option.description}</span>
              </button>
            ))}
          </div>

          {isEmbedding && (
            <p className="settings-privacy-note">
              {config.provider === 'ollama'
                ? 'Entity names and their selected context stay on this device.'
                : `Entity names and their selected context are sent to ${activeProvider.name} to create vectors.`}
            </p>
          )}
        </section>

        <section className="settings-subsection">
          <header className="settings-section-head">
            <h3>Connection</h3>
            <p>Endpoint, timeout, and credentials.</p>
          </header>

          <div className="settings-field">
            <label htmlFor={`${prefix}-base-url`}>Base URL</label>
            <input
              id={`${prefix}-base-url`}
              className="settings-input"
              type="url"
              value={config.baseUrl}
              onChange={(event) => changeBaseUrl(event.target.value)}
              placeholder={activeProvider.defaultBaseUrl}
              autoComplete="url"
            />
          </div>

          <div className="settings-field settings-field-compact">
            <label htmlFor={`${prefix}-timeout-secs`}>Timeout (seconds)</label>
            <input
              id={`${prefix}-timeout-secs`}
              className="settings-input"
              type="number"
              min={1}
              step={1}
              value={config.timeoutSecs}
              onChange={(event) => update({ timeoutSecs: event.target.value })}
              placeholder="60"
              inputMode="numeric"
            />
          </div>

          {config.provider !== 'ollama' && (
            <div className="settings-field">
              <label htmlFor={`${prefix}-api-key`}>{activeProvider.name} API key</label>
              <p className="settings-help-text">Stored with the rest of your local ARKA settings.</p>
              <input
                id={`${prefix}-api-key`}
                className="settings-input"
                type="password"
                value={config.apiKey}
                onChange={(event) => update({ apiKey: event.target.value })}
                placeholder={config.provider === 'openai' ? 'sk-...' : 'sk-or-v1-...'}
                autoComplete="off"
                spellCheck={false}
              />
            </div>
          )}
        </section>

        <section className={`settings-subsection${isEmbedding ? ' settings-subsection-last' : ''}`}>
          <header className="settings-section-head">
            <h3>Model</h3>
            <p>
              {config.provider === 'ollama'
                ? 'Fetch installed Ollama models, then choose one.'
                : isEmbedding
                  ? `Enter an embedding model ID available from ${activeProvider.name}.`
                  : `Enter a generation model ID available from ${activeProvider.name}.`}
            </p>
          </header>

          {config.provider === 'ollama' ? (
            <div className="settings-field">
              <label htmlFor={`${prefix}-selected-model`}>Model</label>
              <div className="model-row">
                <select
                  id={`${prefix}-selected-model`}
                  className="settings-input model-input"
                  value={config.modelId}
                  onChange={(event) => update({ modelId: event.target.value })}
                >
                  <option value="">Select a fetched model</option>
                  {availableModels.map((item) => (
                    <option key={item} value={item}>
                      {item}
                    </option>
                  ))}
                </select>

                <button
                  type="button"
                  className="btn-secondary settings-fetch-btn"
                  onClick={onFetchModels}
                  disabled={isFetching}
                >
                  {isFetching ? 'Fetching...' : 'Fetch models'}
                </button>
              </div>
            </div>
          ) : (
            <div className="settings-field">
              <label htmlFor={`${prefix}-model-id`}>{activeProvider.name} model ID</label>
              <input
                id={`${prefix}-model-id`}
                className="settings-input"
                type="text"
                value={config.modelId}
                onChange={(event) => update({ modelId: event.target.value })}
                placeholder={modelPrompt(scope, config.provider)}
                autoComplete="off"
                spellCheck={false}
              />
            </div>
          )}

          {fetchStatus && (
            <p className={statusClassName(fetchStatus)} role={fetchStatus.tone === 'error' ? 'alert' : 'status'}>
              {fetchStatus.message}
            </p>
          )}
        </section>

        {!isEmbedding && llmConcurrency !== undefined && onLlmConcurrencyChange && (
          <section className="settings-subsection settings-subsection-last">
            <header className="settings-section-head">
              <h3>Parallel requests</h3>
              <p>Set one request limit for all language-model work.</p>
            </header>

            <div className="settings-field settings-concurrency-field">
              <label htmlFor="llm-concurrency">LLM concurrency</label>
              <input
                id="llm-concurrency"
                className="settings-input settings-concurrency-input"
                type="number"
                min={1}
                max={MAX_LLM_CONCURRENCY}
                step={1}
                value={llmConcurrency}
                onChange={(event) => {
                  onLlmConcurrencyChange(event.target.value)
                  onClearStatus()
                }}
                aria-describedby="llm-concurrency-help"
                inputMode="numeric"
              />
              <p id="llm-concurrency-help" className="settings-help-text">
                Start with 5. Use 1–2 for local models or limited hardware; try 6–8 for hosted APIs.
                Lower this value if requests slow down or you see rate-limit (429) errors. Higher
                values use more provider capacity and may not finish faster.
              </p>
            </div>
          </section>
        )}
      </div>
    </section>
  )
}

export function ModelsPage() {
  const [savedGenerationConfig, setSavedGenerationConfig] = useState<ProviderConfig | null>(null)
  const [savedEmbeddingConfig, setSavedEmbeddingConfig] = useState<ProviderConfig | null>(null)
  const [generationConfig, setGenerationConfig] = useState<ProviderConfig>(() => defaultConfig())
  const [embeddingConfig, setEmbeddingConfig] = useState<ProviderConfig>(() => defaultConfig())
  const [llmConcurrency, setLlmConcurrency] = useState(String(DEFAULT_LLM_CONCURRENCY))
  const [modelHistory, setModelHistory] = useState<ModelHistoryMap>(() => readModelHistory())
  const [fetchingScope, setFetchingScope] = useState<ConfigScope | null>(null)
  const [fetchStatuses, setFetchStatuses] = useState<Partial<Record<ConfigScope, StatusMessage>>>({})
  const [pageStatus, setPageStatus] = useState<StatusMessage | null>(null)
  const [testStatus, setTestStatus] = useState<StatusMessage | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [isTestingEmbedding, setIsTestingEmbedding] = useState(false)

  useEffect(() => {
    invoke<DbModelConfig>('load_model_config')
      .then((config) => {
        const loadedGeneration = formConfig(config)
        const loadedEmbedding = embeddingFormConfig(config)
        setSavedGenerationConfig(loadedGeneration)
        setSavedEmbeddingConfig(loadedEmbedding)
        setGenerationConfig(loadedGeneration)
        setEmbeddingConfig(loadedEmbedding)
        setLlmConcurrency(String(config.llm_concurrency ?? DEFAULT_LLM_CONCURRENCY))
      })
      .catch((err) => {
        console.error('Failed to load model config from DB:', err)
        setPageStatus({
          message: 'Model settings could not be loaded. Check the application logs and try again.',
          tone: 'error',
        })
      })
  }, [])

  const clearFetchStatus = (scope: ConfigScope) => {
    setFetchStatuses((current) => ({ ...current, [scope]: undefined }))
  }

  const updateGenerationConfig = (config: ProviderConfig) => {
    setGenerationConfig(config)
    setPageStatus(null)
  }

  const updateEmbeddingConfig = (config: ProviderConfig) => {
    setEmbeddingConfig(config)
    setPageStatus(null)
    setTestStatus(null)
  }

  const fetchModels = async (scope: ConfigScope) => {
    const config = scope === 'generation' ? generationConfig : embeddingConfig
    const timeoutSecs = Number(config.timeoutSecs)

    if (config.provider !== 'ollama') {
      setFetchStatuses((current) => ({
        ...current,
        [scope]: { message: 'Model fetching is available for Ollama connections.', tone: 'error' },
      }))
      return
    }

    if (!config.baseUrl.trim()) {
      setFetchStatuses((current) => ({
        ...current,
        [scope]: { message: 'Enter the Ollama base URL before fetching models.', tone: 'error' },
      }))
      return
    }

    if (!Number.isInteger(timeoutSecs) || timeoutSecs <= 0) {
      setFetchStatuses((current) => ({
        ...current,
        [scope]: { message: 'Timeout must be a whole number greater than 0.', tone: 'error' },
      }))
      return
    }

    setFetchingScope(scope)
    setFetchStatuses((current) => ({
      ...current,
      [scope]: { message: 'Fetching installed models...', tone: 'info' },
    }))

    try {
      const fetchedModels = await invoke<string[]>('fetch_ollama_models', {
        baseUrl: config.baseUrl.trim(),
        modelName: null,
        timeoutSecs,
      })
      const key = getHistoryKey(config.provider, config.baseUrl)
      const mergedModels = dedupeModels([...(modelHistory[key] ?? []), ...fetchedModels])
      const nextHistory = { ...modelHistory, [key]: mergedModels }
      setModelHistory(nextHistory)
      writeModelHistory(nextHistory)

      if (fetchedModels.length > 0) {
        const nextModel = config.modelId && mergedModels.includes(config.modelId)
          ? config.modelId
          : fetchedModels[0]
        if (scope === 'generation') {
          setGenerationConfig((current) => ({ ...current, modelId: nextModel }))
        } else {
          setEmbeddingConfig((current) => ({ ...current, modelId: nextModel }))
        }
      }

      setFetchStatuses((current) => ({
        ...current,
        [scope]: fetchedModels.length === 0
          ? { message: 'No installed models were returned by Ollama.', tone: 'error' }
          : { message: `Found ${fetchedModels.length} installed model${fetchedModels.length === 1 ? '' : 's'}.`, tone: 'success' },
      }))
    } catch (err) {
      setFetchStatuses((current) => ({
        ...current,
        [scope]: {
          message: errorMessage(err, 'Failed to fetch Ollama models.'),
          tone: 'error',
        },
      }))
    } finally {
      setFetchingScope(null)
    }
  }

  const testEmbeddingConnection = async () => {
    const normalized = normalizedConfig(embeddingConfig, 'Embedding')
    if (isStatusMessage(normalized)) {
      setTestStatus(normalized)
      return
    }

    setIsTestingEmbedding(true)
    setTestStatus({ message: 'Testing the embedding connection...', tone: 'info' })
    try {
      const result = await invoke<EmbeddingConnectionResult>('test_embedding_config', {
        config: normalized,
      })
      setTestStatus({
        message: `Connection successful. ${result.model} returned ${result.dimensions} dimensions.`,
        tone: 'success',
      })
    } catch (err) {
      setTestStatus({
        message: errorMessage(err, 'Embedding connection test failed.'),
        tone: 'error',
      })
    } finally {
      setIsTestingEmbedding(false)
    }
  }

  const saveModelSettings = async () => {
    setPageStatus(null)
    const generation = normalizedConfig(generationConfig, 'Question generation')
    if (isStatusMessage(generation)) {
      setPageStatus(generation)
      return
    }

    const embedding = normalizedConfig(embeddingConfig, 'Embedding', true)
    if (isStatusMessage(embedding)) {
      setPageStatus(embedding)
      return
    }

    const concurrency = Number(llmConcurrency)
    if (
      !Number.isInteger(concurrency)
      || concurrency < 1
      || concurrency > MAX_LLM_CONCURRENCY
    ) {
      setPageStatus({
        message: `LLM concurrency must be a whole number between 1 and ${MAX_LLM_CONCURRENCY}.`,
        tone: 'error',
      })
      return
    }

    setIsSaving(true)
    try {
      await invoke('set_runtime_llm_settings', {
        provider: generation.provider,
        baseUrl: generation.base_url,
        model: generation.selected_model,
        timeoutSecs: generation.timeout_secs,
        apiKey: generation.api_key,
      })
      await invoke('save_model_config', {
        config: {
          ...generation,
          llm_concurrency: concurrency,
          embedding_provider: embedding.provider,
          embedding_base_url: embedding.base_url,
          embedding_selected_model: embedding.selected_model,
          embedding_timeout_secs: embedding.timeout_secs,
          embedding_api_key: embedding.api_key,
        },
      })

      const savedGeneration = formConfig(generation)
      const savedEmbedding = formConfig(embedding)
      setSavedGenerationConfig(savedGeneration)
      setSavedEmbeddingConfig(savedEmbedding)
      setGenerationConfig(savedGeneration)
      setEmbeddingConfig(savedEmbedding)
      setLlmConcurrency(String(concurrency))
      setPageStatus(
        embedding.selected_model
          ? { message: 'Model and LLM concurrency settings saved.', tone: 'success' }
          : {
              message: 'Generation and LLM concurrency settings saved. Entity embeddings remain unconfigured.',
              tone: 'info',
            },
      )
    } catch (err) {
      setPageStatus({
        message: errorMessage(err, 'Failed to save model settings.'),
        tone: 'error',
      })
    } finally {
      setIsSaving(false)
    }
  }

  return (
    <div className="app-container settings-page" aria-label="Models page">
      <header className="settings-panel settings-page-intro">
        <h1>Model settings</h1>
        <p className="settings-help-text">
          Configure language-model generation, parallel requests, and entity matching.
        </p>
      </header>

      <ModelConfigPanel
        scope="generation"
        title="Question generation"
        description="The language model that extracts knowledge and writes recall questions."
        config={generationConfig}
        savedConfig={savedGenerationConfig}
        modelHistory={modelHistory}
        fetchStatus={fetchStatuses.generation ?? null}
        isFetching={fetchingScope === 'generation'}
        onChange={updateGenerationConfig}
        onFetchModels={() => fetchModels('generation')}
        onClearStatus={() => clearFetchStatus('generation')}
        llmConcurrency={llmConcurrency}
        onLlmConcurrencyChange={(value) => {
          setLlmConcurrency(value)
          setPageStatus(null)
        }}
      />

      <ModelConfigPanel
        scope="embedding"
        title="Entity embeddings"
        description="The embedding model that finds possible duplicate entities before semantic verification."
        config={embeddingConfig}
        savedConfig={savedEmbeddingConfig}
        modelHistory={modelHistory}
        fetchStatus={fetchStatuses.embedding ?? null}
        isFetching={fetchingScope === 'embedding'}
        onChange={updateEmbeddingConfig}
        onFetchModels={() => fetchModels('embedding')}
        onClearStatus={() => clearFetchStatus('embedding')}
      />

      <div className="embedding-test-row">
        <div>
          <h2>Verify entity embeddings</h2>
          <p>Send one short test input and confirm the model returns a valid vector.</p>
        </div>
        <button
          type="button"
          className="btn-secondary"
          onClick={testEmbeddingConnection}
          disabled={isTestingEmbedding || isSaving}
        >
          {isTestingEmbedding ? 'Testing...' : 'Test embedding'}
        </button>
      </div>

      {testStatus && (
        <p className={statusClassName(testStatus)} role={testStatus.tone === 'error' ? 'alert' : 'status'}>
          {testStatus.message}
        </p>
      )}

      <div className="settings-actions settings-actions-right model-settings-save-row">
        <button
          type="button"
          className="btn-primary"
          onClick={saveModelSettings}
          disabled={isSaving || isTestingEmbedding}
        >
          {isSaving ? 'Saving...' : 'Save model settings'}
        </button>

        {pageStatus && (
          <p className={statusClassName(pageStatus)} role={pageStatus.tone === 'error' ? 'alert' : 'status'}>
            {pageStatus.message}
          </p>
        )}
      </div>
    </div>
  )
}
