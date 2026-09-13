import type { RecallItem } from '../../learning-items/recall'
import { AnswerOption } from './AnswerOption'
import { GenerationModel } from './GenerationModel'

type QuestionCardProps = {
  question: Extract<RecallItem, { format: 'mcq' }>
  selectedOptionId: string | null
  isSubmitted: boolean
  isSubmitting: boolean
  onSelectOption: (optionId: string) => void
  onSubmit: () => void
}

export function QuestionCard({
  question,
  selectedOptionId,
  isSubmitted,
  isSubmitting,
  onSelectOption,
  onSubmit,
}: QuestionCardProps) {
  const canSubmit = selectedOptionId !== null && !isSubmitted && !isSubmitting
  const outcome = isSubmitted
    ? selectedOptionId === question.correct_option_id ? 'recalled' : 'again'
    : undefined

  return (
    <section className="session-question-card surface-panel" aria-label="Question card" data-outcome={outcome}>
      <div className="session-question-head">
        <p className="session-question-kicker">Review prompt</p>
        <h2>{question.prompt}</h2>
        <GenerationModel model={question.model} />
      </div>

      <div className="session-answer-grid" role="list" aria-label="Answer options">
        {question.options.map((option, index) => {
          const isSelected = selectedOptionId === option.id
          const isCorrect = isSubmitted && option.id === question.correct_option_id
          const isIncorrect =
            isSubmitted && isSelected && option.id !== question.correct_option_id

          return (
            <div key={option.id} role="listitem">
              <AnswerOption
                id={option.id}
                label={String.fromCharCode(65 + index)}
                text={option.text}
                isSelected={isSelected}
                isSubmitted={isSubmitted}
                disabled={isSubmitted || isSubmitting}
                isCorrect={isCorrect}
                isIncorrect={isIncorrect}
                onSelect={onSelectOption}
              />
            </div>
          )
        })}
      </div>

      <div className="session-question-actions">
        <button
          type="button"
          className="btn-primary session-submit-btn"
          onClick={onSubmit}
          disabled={!canSubmit}
        >
          {isSubmitting ? 'Submitting...' : isSubmitted ? 'Answer submitted' : 'Submit answer'}
        </button>
      </div>
    </section>
  )
}
