import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ChevronRight, Copy, Minus, Pause, Sparkles, Square, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import arkaAppIcon from '../assets/arka-app-icon.png'
import { useGeneration } from '../generation/context'
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
  const { generationProgress, isGenerating } = useGeneration()
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
  const generationPercent = Math.min(
    100,
    Math.max(0, Math.round(generationProgress?.progress_percent ?? 0)),
  )
  const isGenerationPaused = Boolean(generationProgress?.is_paused)

  return (
    <header className="app-titlebar" data-tauri-drag-region>
      <div className="titlebar-brand" data-tauri-drag-region>
        <img src={arkaAppIcon} alt="" className="titlebar-logo" draggable={false} />
        <span data-tauri-drag-region>A.R.K.A.</span>
      </div>

      {isGenerating ? (
        <Link
          className={`titlebar-generation-status${isGenerationPaused ? ' is-paused' : ''}`}
          to="/#generation-progress"
          aria-label={`${isGenerationPaused ? 'Generation paused' : 'Generation in progress'}, ${generationPercent}% complete. Return to generation.`}
        >
          <span className="titlebar-generation-icon" aria-hidden="true">
            {isGenerationPaused ? <Pause /> : <Sparkles />}
          </span>
          <span className="titlebar-generation-copy" aria-hidden="true">
            <span>{isGenerationPaused ? 'Paused' : 'Generating'}</span>
            <strong>{generationPercent}%</strong>
          </span>
          <progress
            className="titlebar-generation-progress"
            max="100"
            value={generationPercent}
            aria-hidden="true"
          />
          <span className="titlebar-generation-return" aria-hidden="true">
            View
            <ChevronRight />
          </span>
        </Link>
      ) : (
        <nav
          className="titlebar-config"
          data-tauri-drag-region
          aria-label="Current model configuration"
          aria-busy={isLoading}
        >
          <Link
            className={`titlebar-config-item${!isLoading && config.generation === 'Not configured' ? ' is-unconfigured' : ''}`}
            to="/models#generation-model-settings"
            aria-label={`Question generation model: ${isLoading ? 'Loading' : config.generation}. Open model settings.`}
            aria-describedby="titlebar-generation-help"
          >
            <span className="titlebar-config-label">LLM</span>
            <span className="titlebar-config-value">
              {isLoading ? 'Loading…' : config.generation}
            </span>
            <ChevronRight className="titlebar-config-chevron" aria-hidden="true" />
            <span id="titlebar-generation-help" className="titlebar-config-tooltip" role="tooltip">
              <strong>Question generation</strong>
              <span>Creates flashcards and multiple-choice questions from your notes.</span>
            </span>
          </Link>
          <span className="titlebar-config-divider" aria-hidden="true" data-tauri-drag-region />
          <Link
            className={`titlebar-config-item${!isLoading && config.embedding === 'Not configured' ? ' is-unconfigured' : ''}`}
            to="/models#embedding-model-settings"
            aria-label={`Embedding model: ${isLoading ? 'Loading' : config.embedding}. Open model settings.`}
            aria-describedby="titlebar-embedding-help"
          >
            <span className="titlebar-config-label">Embedding</span>
            <span className="titlebar-config-value">
              {isLoading ? 'Loading…' : config.embedding}
            </span>
            <ChevronRight className="titlebar-config-chevron" aria-hidden="true" />
            <span id="titlebar-embedding-help" className="titlebar-config-tooltip" role="tooltip">
              <strong>Entity embeddings</strong>
              <span>Powers Deep thinking by finding related concepts across your note.</span>
            </span>
          </Link>
        </nav>
      )}

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
