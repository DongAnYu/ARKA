export function GenerationModel({ model }: { model: string | null }) {
  if (!model?.trim()) return null

  return (
    <div className="learning-item-review-model" aria-label={`Generation model: ${model}`} title={`LLM: ${model}`}>
      <span className="learning-item-review-model-label">LLM</span>
      <span className="learning-item-review-model-value">{model}</span>
    </div>
  )
}
