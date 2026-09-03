import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Copy, Minus, Square, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import arkaLogo from '../assets/arka-logo.svg'
import {
  MODEL_CONFIG_UPDATED_EVENT,
  type PersistedModelConfig,
} from '../modelConfig'

type ConfigSummary = {
  generation: string
  embedding: string
}

const EMPTY_CONFIG: ConfigSummary = {
  generation: 'Not configured',
  embedding: 'Not configured',
}

const RUNNING_IN_TAURI = isTauri()

const providerName = (provider: string) => {
  const names: Record<string, string> = {
    ollama: 'Ollama',
    openai: 'OpenAI',
    openrouter: 'OpenRouter',
  }

  return names[provider.toLowerCase()] ?? provider
}

const formatModel = (provider: string, model: string) => {
  const trimmedModel = model.trim()
  return trimmedModel ? `${providerName(provider)} · ${trimmedModel}` : 'Not configured'
}

const summarizeConfig = (config: PersistedModelConfig): ConfigSummary => ({
  generation: formatModel(config.provider, config.selected_model),
  embedding: formatModel(config.embedding_provider, config.embedding_selected_model),
})

export function AppTitleBar() {
  const [config, setConfig] = useState<ConfigSummary>(EMPTY_CONFIG)
  const [isLoading, setIsLoading] = useState(RUNNING_IN_TAURI)
  const [isMaximized, setIsMaximized] = useState(false)

  useEffect(() => {
    if (!RUNNING_IN_TAURI) {
      return
    }

    let isMounted = true

    const loadConfig = () => {
      invoke<PersistedModelConfig>('load_model_config')
        .then((loadedConfig) => {
          if (isMounted) {
            setConfig(summarizeConfig(loadedConfig))
          }
        })
        .catch((error) => {
          console.error('Failed to load title bar model config:', error)
        })
        .finally(() => {
          if (isMounted) {
            setIsLoading(false)
          }
        })
    }

    const handleConfigUpdate = (event: Event) => {
      const updatedConfig = (event as CustomEvent<PersistedModelConfig>).detail
      setConfig(summarizeConfig(updatedConfig))
      setIsLoading(false)
    }

    const handleFocus = () => loadConfig()

    loadConfig()
    window.addEventListener(MODEL_CONFIG_UPDATED_EVENT, handleConfigUpdate)
    window.addEventListener('focus', handleFocus)

    return () => {
      isMounted = false
      window.removeEventListener(MODEL_CONFIG_UPDATED_EVENT, handleConfigUpdate)
      window.removeEventListener('focus', handleFocus)
    }
  }, [])

  useEffect(() => {
    if (!RUNNING_IN_TAURI) {
      return
    }

    const appWindow = getCurrentWindow()
    let unlistenResize: (() => void) | undefined

    const syncMaximizedState = () => {
      appWindow.isMaximized().then(setIsMaximized).catch(() => setIsMaximized(false))
    }

    syncMaximizedState()
    appWindow.onResized(syncMaximizedState).then((unlisten) => {
      unlistenResize = unlisten
    }).catch(() => undefined)

    return () => unlistenResize?.()
  }, [])

  const runWindowAction = (action: () => Promise<void>) => {
    action().catch((error) => console.error('Window action failed:', error))
  }

  const appWindow = RUNNING_IN_TAURI ? getCurrentWindow() : null

  return (
    <header className="app-titlebar" data-tauri-drag-region>
      <div className="titlebar-brand" data-tauri-drag-region>
        <img src={arkaLogo} alt="" className="titlebar-logo" draggable={false} />
        <span data-tauri-drag-region>A.R.K.A</span>
      </div>

      <div
        className="titlebar-config"
        data-tauri-drag-region
        aria-label="Current model configuration"
        aria-busy={isLoading}
      >
        <div className="titlebar-config-item" data-tauri-drag-region title={`LLM: ${config.generation}`}>
          <span className="titlebar-config-label" data-tauri-drag-region>LLM</span>
          <span className="titlebar-config-value" data-tauri-drag-region>
            {isLoading ? 'Loading…' : config.generation}
          </span>
        </div>
        <span className="titlebar-config-divider" aria-hidden="true" data-tauri-drag-region />
        <div className="titlebar-config-item" data-tauri-drag-region title={`Embedding: ${config.embedding}`}>
          <span className="titlebar-config-label" data-tauri-drag-region>Embedding</span>
          <span className="titlebar-config-value" data-tauri-drag-region>
            {isLoading ? 'Loading…' : config.embedding}
          </span>
        </div>
      </div>

      <div className="titlebar-controls" aria-label="Window controls">
        <button
          type="button"
          className="titlebar-control"
          aria-label="Minimize window"
          onClick={() => appWindow && runWindowAction(() => appWindow.minimize())}
        >
          <Minus aria-hidden="true" />
        </button>
        <button
          type="button"
          className="titlebar-control"
          aria-label={isMaximized ? 'Restore window' : 'Maximize window'}
          onClick={() => appWindow && runWindowAction(() => appWindow.toggleMaximize())}
        >
          {isMaximized ? <Copy aria-hidden="true" /> : <Square aria-hidden="true" />}
        </button>
        <button
          type="button"
          className="titlebar-control titlebar-control-close"
          aria-label="Close window"
          onClick={() => appWindow && runWindowAction(() => appWindow.close())}
        >
          <X aria-hidden="true" />
        </button>
      </div>
    </header>
  )
}
