import { CreditCard, Info, ListChecks, X } from 'lucide-react'
import type { LearningItemEditorValue } from '../learning-items/editor'

type LearningItemEditorProps = {
  value: LearningItemEditorValue
  onChange: (value: LearningItemEditorValue) => void
  disabled?: boolean
  idPrefix: string
  flashcardRequired?: boolean
  editMcqOmissionReason?: boolean
}

export function LearningItemEditor({
  value,
  onChange,
  disabled = false,
  idPrefix,
  flashcardRequired = false,
  editMcqOmissionReason = false,
}: LearningItemEditorProps) {
  const update = (updates: Partial<LearningItemEditorValue>) => {
    onChange({ ...value, ...updates })
  }
  const distractors = value.mcq?.distractors ?? []
  const coreTitleId = `${idPrefix}-core-knowledge-title`
  const variantsTitleId = `${idPrefix}-practice-variants-title`

  return (
    <div className="learning-item-review-columns">
      <section className="learning-item-review-panel" aria-labelledby={coreTitleId}>
        <div className="learning-item-review-section-heading">
          <h4 id={coreTitleId}>Core knowledge</h4>
          <p>Shared content for every practice variant in this learning item.</p>
        </div>

        <div className="learning-item-review-fields">
          <label className="question-review-field">
            <span>
              Learning target {!value.flashcard && <small>(optional for MCQ-only items)</small>}
            </span>
            <textarea
              className="edit-space-input edit-space-textarea"
              value={value.target}
              onChange={(event) => update({ target: event.target.value })}
              rows={3}
              disabled={disabled}
            />
          </label>

          <label className="question-review-field">
            <span>Answer</span>
            <textarea
              className="edit-space-input edit-space-textarea"
              value={value.answer}
              onChange={(event) => update({ answer: event.target.value })}
              rows={4}
              disabled={disabled}
            />
          </label>

          <label className="question-review-field">
            <span>
              Explanation <small>(optional)</small>
            </span>
            <textarea
              className="edit-space-input edit-space-textarea"
              value={value.explanation ?? ''}
              onChange={(event) => update({ explanation: event.target.value || null })}
              rows={4}
              disabled={disabled}
            />
          </label>
        </div>
      </section>

      <section className="learning-item-review-panel" aria-labelledby={variantsTitleId}>
        <div className="learning-item-review-section-heading">
          <h4 id={variantsTitleId}>Practice variants</h4>
          <p>Different formats for active recall, using the same core knowledge.</p>
        </div>

        {value.flashcard ? (
          <article className="learning-item-variant-card">
            <div className="learning-item-variant-heading">
              <CreditCard aria-hidden="true" />
              <strong>Flashcard</strong>
              <span className={`learning-item-variant-badge${flashcardRequired ? ' is-required' : ''}`}>
                {flashcardRequired ? 'Required' : 'Available'}
              </span>
            </div>
            <label className="question-review-field">
              <span>Prompt</span>
              <textarea
                className="edit-space-input edit-space-textarea"
                value={value.flashcard.prompt}
                onChange={(event) => update({ flashcard: { prompt: event.target.value } })}
                rows={2}
                disabled={disabled}
              />
            </label>
            <p className="learning-item-shared-answer-note">
              <Info aria-hidden="true" />
              Uses the shared answer on the left. The answer is not duplicated here.
            </p>
          </article>
        ) : (
          <article className="learning-item-variant-card is-omitted">
            <div className="learning-item-variant-heading">
              <X aria-hidden="true" />
              <strong>Flashcard unavailable</strong>
              <span className="learning-item-variant-badge">Legacy item</span>
            </div>
          </article>
        )}

        {value.mcq ? (
          <article className="learning-item-variant-card">
            <div className="learning-item-variant-heading">
              <ListChecks aria-hidden="true" />
              <strong>Multiple choice</strong>
              <span className="learning-item-variant-badge">Optional</span>
            </div>
            <label className="question-review-field">
              <span>Question</span>
              <textarea
                className="edit-space-input edit-space-textarea"
                value={value.mcq.prompt}
                onChange={(event) =>
                  update({ mcq: value.mcq ? { ...value.mcq, prompt: event.target.value } : null })
                }
                rows={2}
                disabled={disabled}
              />
            </label>
            <fieldset className="learning-item-mcq-options">
              <legend>Answer options</legend>
              <div className="is-correct">
                <input
                  type="radio"
                  name={`correct-answer-${idPrefix}`}
                  checked
                  readOnly
                  aria-label="Shared answer is correct"
                />
                <span>Correct</span>
                <p title="Edit this value in the shared Answer field">{value.answer}</p>
              </div>
              {Array.from({ length: 3 }, (_, distractorIndex) => (
                <label key={`${idPrefix}-distractor-${distractorIndex}`}>
                  <input
                    type="radio"
                    name={`correct-answer-${idPrefix}`}
                    checked={false}
                    onChange={() => {
                      if (!value.mcq) return
                      const nextAnswer = distractors[distractorIndex] ?? ''
                      const nextDistractors = [...distractors]
                      nextDistractors[distractorIndex] = value.answer
                      update({
                        answer: nextAnswer,
                        mcq: { ...value.mcq, distractors: nextDistractors },
                      })
                    }}
                    aria-label={`Make distractor ${distractorIndex + 1} the correct answer`}
                    disabled={disabled}
                  />
                  <span>{distractorIndex + 1}</span>
                  <textarea
                    className="edit-space-input edit-question-option-textarea"
                    value={distractors[distractorIndex] ?? ''}
                    onChange={(event) => {
                      if (!value.mcq) return
                      const nextDistractors = [...distractors]
                      nextDistractors[distractorIndex] = event.target.value
                      update({
                        mcq: { ...value.mcq, distractors: nextDistractors },
                      })
                    }}
                    rows={1}
                    aria-label={`Distractor ${distractorIndex + 1}`}
                    disabled={disabled}
                  />
                </label>
              ))}
              <small>The answer order is mixed when the item is saved.</small>
            </fieldset>
          </article>
        ) : (
          <article className="learning-item-variant-card is-omitted">
            <div className="learning-item-variant-heading">
              <X aria-hidden="true" />
              <strong>Multiple choice omitted</strong>
            </div>
            {editMcqOmissionReason ? (
              <label className="question-review-field">
                <span>Reason</span>
                <textarea
                  className="edit-space-input edit-space-textarea"
                  value={value.mcqOmissionReason ?? ''}
                  onChange={(event) => update({ mcqOmissionReason: event.target.value })}
                  rows={3}
                  disabled={disabled}
                />
              </label>
            ) : (
              <p className="learning-item-shared-answer-note">
                This saved item does not include a multiple-choice variant.
              </p>
            )}
          </article>
        )}
      </section>
    </div>
  )
}
