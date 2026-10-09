import {
  isGenerationPurpose,
  LEARNING_PURPOSES,
  MAX_LEARNING_ITEMS,
  maximumInputError,
} from '../options'
import type { GenerationOptions } from '../types'

type DefaultGenerationOptionsProps = {
  options: GenerationOptions
  disabled: boolean
  maximumInput: string
  maximumError: string | null
  onMaximumInputChange: (value: string) => void
  onOptionsChange: (options: GenerationOptions) => void
}

export function DefaultGenerationOptions({
  options,
  disabled,
  maximumInput,
  maximumError,
  onMaximumInputChange,
  onOptionsChange,
}: DefaultGenerationOptionsProps) {
  return (
    <fieldset className="generation-options" disabled={disabled}>
      <legend>Shape your study set</legend>
      <div className="generation-options-fields">
        <div className="generation-option-field">
          <label htmlFor="generation-maximum">Maximum learning items</label>
          <input
            id="generation-maximum"
            type="number"
            min={1}
            max={MAX_LEARNING_ITEMS}
            step={1}
            required
            value={maximumInput}
            aria-invalid={Boolean(maximumError)}
            aria-describedby={`generation-maximum-help${maximumError ? ' generation-maximum-error' : ''}`}
            onChange={(event) => {
              const value = event.target.value
              onMaximumInputChange(value)
              if (!maximumInputError(value)) {
                onOptionsChange({ ...options, max_learning_items: Number(value) })
              }
            }}
          />
          <small id="generation-maximum-help">Across the entire selected note.</small>
          {maximumError && <p className="error-text" id="generation-maximum-error" role="alert">{maximumError}</p>}
        </div>
        <div className="generation-option-field">
          <label htmlFor="generation-purpose">Learning purpose</label>
          <select
            id="generation-purpose"
            value={options.purpose}
            aria-describedby="generation-purpose-help"
            onChange={(event) => {
              const purpose = event.target.value
              if (isGenerationPurpose(purpose)) {
                onOptionsChange({ ...options, purpose })
              }
            }}
          >
            {Object.entries(LEARNING_PURPOSES).map(([value, purpose]) => (
              <option key={value} value={value}>{purpose.label}</option>
            ))}
          </select>
          <small id="generation-purpose-help">{LEARNING_PURPOSES[options.purpose].description}</small>
        </div>
      </div>
      <p>A flashcard and its optional MCQ count as one learning item. You may receive fewer items when eligible concepts or generation cannot fill the maximum.</p>
    </fieldset>
  )
}
