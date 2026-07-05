import { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

type ProviderId = 'ollama' | 'openrouter'
type ModelHistoryMap = Record<string, string[]>

type ProviderOption = {
  id: ProviderId
  name: string
  description: string
  defaultBaseUrl: string
}

type DbModelConfig = {
  provider: string
  base_url: string
  selected_model: string
  timeout_secs: number
}

const MODEL_HISTORY_KEY = 'models-page-fetch-history-v1'

const providerOptions: ProviderOption[] = [
  {
    id: 'ollama',
    name: 'Ollama',
    description: 'Local models, no API key',
    defaultBaseUrl: 'http://localhost:11434',
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

const clearModelHistoryForKey = (provider: ProviderId, baseUrl: string) => {
  const history = readModelHistory()
  const key = getHistoryKey(provider, baseUrl)
  delete history[key]
  writeModelHistory(history)
}

export function ModelsPage() {
  const isInitialLoadDone = useRef(false)
  const [provider, setProvider] = useState<ProviderId>('ollama')
  const [baseUrl, setBaseUrl] = useState(findProvider('ollama').defaultBaseUrl)
  const [timeoutSecs, setTimeoutSecs] = useState('60')
  const [modelQuery, setModelQuery] = useState('')
  const [selectedModel, setSelectedModel] = useState('')
  const [availableModels, setAvailableModels] = useState<string[]>([])
  const [isFetchingModels, setIsFetchingModels] = useState(false)
  const [fetchStatus, setFetchStatus] = useState('')

  const activeProvider = useMemo(() => findProvider(provider), [provider])

  // Load saved config from DB on mount
  useEffect(() => {
    invoke<DbModelConfig>('load_model_config')
      .then((config) => {
        const savedProvider = (config.provider === 'openrouter' ? 'openrouter' : 'ollama') as ProviderId
        setProvider(savedProvider)
        setBaseUrl(config.base_url)
        setTimeoutSecs(String(config.timeout_secs))
        setSelectedModel(config.selected_model)
        isInitialLoadDone.current = true
      })
      .catch((err) => {
        console.error('Failed to load model config from DB:', err)
        isInitialLoadDone.current = true
      })
  }, [])

  // Restore fetched model list when provider/baseUrl changes.
  // Only clear selectedModel after the initial DB load, so the restored selection is preserved.
  useEffect(() => {
    const history = readModelHistory()
    const key = getHistoryKey(provider, baseUrl)
    const models = history[key] ?? []
    setAvailableModels(models)
    if (isInitialLoadDone.current) {
      setSelectedModel((current) => (models.includes(current) ? current : ''))
    }
  }, [provider, baseUrl])

  useEffect(() => {
    const normalizedBaseUrl = baseUrl.trim()
    const normalizedModel = selectedModel.trim()
    const parsedTimeout = Number(timeoutSecs)
    const isValidTimeout = Number.isInteger(parsedTimeout) && parsedTimeout > 0

    if (provider !== 'ollama' || !normalizedBaseUrl || !normalizedModel || !isValidTimeout) {
      return
    }

    invoke('save_model_config', {
      config: {
        provider,
        base_url: normalizedBaseUrl,
        selected_model: normalizedModel,
        timeout_secs: parsedTimeout,
      },
    }).catch((err) => {
      console.error('Failed to save model config to DB:', err)
    })

    invoke('set_runtime_llm_settings', {
      provider,
      baseUrl: normalizedBaseUrl,
      model: normalizedModel,
      timeoutSecs: parsedTimeout,
    }).catch((err) => {
      console.error('Failed to sync runtime LLM settings:', err)
    })
  }, [provider, baseUrl, selectedModel, timeoutSecs])

  const persistHistory = (models: string[]) => {
    const normalized = dedupeModels(models)
    setAvailableModels(normalized)

    const history = readModelHistory()
    const key = getHistoryKey(provider, baseUrl)
    history[key] = normalized
    writeModelHistory(history)
  }

  const selectProvider = (nextProvider: ProviderId) => {
    const config = findProvider(nextProvider)
    setProvider(nextProvider)
    setBaseUrl(config.defaultBaseUrl)
    setFetchStatus('')
    setModelQuery('')
    setSelectedModel('')
    invoke('save_model_config', {
      config: {
        provider: nextProvider,
        base_url: config.defaultBaseUrl,
        selected_model: '',
        timeout_secs: Number(timeoutSecs) || 60,
      },
    }).catch((err) => {
      console.error('Failed to save model config to DB:', err)
    })
  }

  const fetchModels = async () => {
    if (provider !== 'ollama') {
      setFetchStatus('Backend fetch currently supports Ollama only.')
      return
    }

    setIsFetchingModels(true)
    setFetchStatus('Fetching models...')

    const parsedTimeout = Number(timeoutSecs)
    if (!Number.isInteger(parsedTimeout) || parsedTimeout <= 0) {
      setFetchStatus('Timeout must be a whole number greater than 0.')
      setIsFetchingModels(false)
      return
    }

    try {
      const fetchedModels = await invoke<string[]>('fetch_ollama_models', {
        baseUrl: baseUrl.trim(),
        modelName: modelQuery.trim() || null,
        timeoutSecs: parsedTimeout,
      })

      const mergedModels = dedupeModels([...availableModels, ...fetchedModels])
      persistHistory(mergedModels)

      if (fetchedModels.length > 0) {
        setSelectedModel((current) => {
          if (current && mergedModels.includes(current)) {
            return current
          }

          return fetchedModels[0]
        })
      }

      if (fetchedModels.length === 0) {
        setFetchStatus('No matching models found from the provider.')
      } else {
        setFetchStatus(`Fetched ${fetchedModels.length} models from ${activeProvider.name}.`)
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to fetch models'
      setFetchStatus(message)
    } finally {
      setIsFetchingModels(false)
    }
  }

  const clearFetchedModels = () => {
    clearModelHistoryForKey(provider, baseUrl)
    setAvailableModels([])
    setSelectedModel('')
    setFetchStatus('Cleared fetched models for this provider and URL.')
  }

  return (
    <div className="app-container settings-page" aria-label="Models page">
      <section className="settings-panel">
        <header className="settings-header">
          <h1>Provider</h1>
          <h2>Choose your LLM source</h2>
          <p>Choose how the assistant connects to a language model.</p>
        </header>

        <div className="provider-grid" role="radiogroup" aria-label="Provider">
          {providerOptions.map((option) => (
            <button
              key={option.id}
              type="button"
              className={`provider-card${provider === option.id ? ' is-active' : ''}`}
              onClick={() => selectProvider(option.id)}
              role="radio"
              aria-checked={provider === option.id}
            >
              <span className="provider-card-title">{option.name}</span>
              <span className="provider-card-description">{option.description}</span>
            </button>
          ))}
        </div>
      </section>

      <section className="settings-panel">
        <header className="settings-section-head">
          <h3>Connection</h3>
          <p>Base URL &amp; timeout</p>
        </header>

        <div className="settings-field">
          <label htmlFor="base-url">Base URL</label>
          <p className="settings-help-text">The HTTP endpoint of your provider.</p>
          <input
            id="base-url"
            className="settings-input"
            type="url"
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
            placeholder={activeProvider.defaultBaseUrl}
          />
        </div>

        <div className="settings-field">
          <label htmlFor="timeout-secs">Timeout (seconds)</label>
          <input
            id="timeout-secs"
            className="settings-input"
            type="number"
            min={1}
            step={1}
            value={timeoutSecs}
            onChange={(event) => setTimeoutSecs(event.target.value)}
            placeholder="60"
          />
        </div>
      </section>

      <section className="settings-panel">
        <header className="settings-section-head">
          <h3>Model</h3>
          <p>Pick the chat model</p>
        </header>

        <p className="settings-help-text">
          Fetch all models, or enter an exact model name to validate it at this URL.
        </p>

        <div className="settings-field">
          <label htmlFor="model-query">Model name to validate (optional)</label>
          <div className="model-row">
            <input
              id="model-query"
              className="settings-input model-input"
              type="text"
              value={modelQuery}
              onChange={(event) => setModelQuery(event.target.value)}
              placeholder="e.g. qwen3:4b"
              autoComplete="off"
            />
            <button
              type="button"
              className="btn-secondary settings-fetch-btn"
              onClick={fetchModels}
              disabled={isFetchingModels}
            >
              {isFetchingModels ? 'Fetching...' : 'Fetch models'}
            </button>
          </div>
        </div>

        <div className="settings-field">
          <label htmlFor="selected-model">Model</label>
          <div className="model-row">
            <select
              id="selected-model"
              className="settings-input model-input"
              value={selectedModel}
              onChange={(event) => setSelectedModel(event.target.value)}
            >
              <option value="">Select fetched model</option>
              {availableModels.map((item) => (
                <option key={item} value={item}>
                  {item}
                </option>
              ))}
            </select>

            <button
              type="button"
              className="btn-secondary settings-fetch-btn"
              onClick={clearFetchedModels}
              disabled={availableModels.length === 0}
            >
              Clear models
            </button>
          </div>
        </div>

        {fetchStatus && <p className="settings-status">{fetchStatus}</p>}
      </section>
    </div>
  )
}
