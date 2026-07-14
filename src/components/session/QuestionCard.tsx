import { AnswerOption } from './AnswerOption'

type OptionId = 'A' | 'B' | 'C' | 'D'

type QuestionOption = {
  id: OptionId
  text: string
}

type SessionQuestion = {
  id: number
  prompt: string
  options: QuestionOption[]
  correctOptionId: OptionId
  explanation?: string
}

type QuestionCardProps = {
  question: SessionQuestion
  selectedOptionId: OptionId | null
  isSubmitted: boolean
  isSubmitting: boolean
  onSelectOption: (optionId: OptionId) => void
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

  return (
    <section className="session-question-card surface-panel" aria-label="Question card">
      <div className="session-question-head">
        <p className="session-question-kicker">Review prompt</p>
        <h2>{question.prompt}</h2>
      </div>

      <div className="session-answer-grid" role="list" aria-label="Answer options">
        {question.options.map((option) => {
          const isSelected = selectedOptionId === option.id
          const isCorrect = isSubmitted && option.id === question.correctOptionId
          const isIncorrect =
            isSubmitted && isSelected && option.id !== question.correctOptionId

          return (
            <div key={option.id} role="listitem">
              <AnswerOption
                id={option.id}
                label={option.id}
                text={option.text}
                isSelected={isSelected}
                isSubmitted={isSubmitted}
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

export type { OptionId, SessionQuestion }
