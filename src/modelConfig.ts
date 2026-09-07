export type PersistedModelConfig = {
  provider: string
  base_url: string
  selected_model: string
  timeout_secs: number
  api_key: string | null
  llm_concurrency: number
  embedding_provider: string
  embedding_base_url: string
  embedding_selected_model: string
  embedding_timeout_secs: number
  embedding_api_key: string | null
}

export const MODEL_CONFIG_UPDATED_EVENT = 'arka:model-config-updated'

export function announceModelConfigUpdate(config: PersistedModelConfig) {
  window.dispatchEvent(
    new CustomEvent<PersistedModelConfig>(MODEL_CONFIG_UPDATED_EVENT, { detail: config }),
  )
}
