import { Check, X } from 'lucide-react'

type OptionId = string

type AnswerOptionProps = {
  id: OptionId
  label: string
  text: string
  isSelected: boolean
  disabled: boolean
  isSubmitted: boolean
  isCorrect: boolean
  isIncorrect: boolean
  onSelect: (id: OptionId) => void
}

export function AnswerOption({
  id,
  label,
  text,
  isSelected,
  isSubmitted,
  disabled,
  isCorrect,
  isIncorrect,
  onSelect,
}: AnswerOptionProps) {
  const classNames = ['session-answer-option']

  if (isSelected) {
    classNames.push('is-selected')
  }

  if (isCorrect) {
    classNames.push('is-correct')
  }

  if (isIncorrect) {
    classNames.push('is-incorrect')
  }

  let statusLabel: string | null = null

  if (isSubmitted && isCorrect) {
    statusLabel = 'Correct answer'
  } else if (isSubmitted && isIncorrect) {
    statusLabel = 'Your choice'
  }

  return (
    <button
      type="button"
      className={classNames.join(' ')}
      onClick={() => onSelect(id)}
      disabled={disabled}
      aria-pressed={isSelected}
      aria-label={statusLabel ? `${label}. ${text}. ${statusLabel}.` : `${label}. ${text}.`}
    >
      <span className="session-answer-label" aria-hidden="true">
        {label}
      </span>
      <span className="session-answer-text">{text}</span>
      {statusLabel ? (
        <span className="session-answer-status">
          {isCorrect ? <Check aria-hidden="true" /> : <X aria-hidden="true" />}
          {statusLabel}
        </span>
      ) : null}
    </button>
  )
}
