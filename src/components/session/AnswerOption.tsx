import type { ReactNode } from 'react'

type OptionId = 'A' | 'B' | 'C' | 'D'

type AnswerOptionProps = {
  id: OptionId
  label: string
  text: string
  isSelected: boolean
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

  let statusLabel: ReactNode = null

  if (isSubmitted && isCorrect) {
    statusLabel = <span className="session-answer-status">Correct answer</span>
  } else if (isSubmitted && isIncorrect) {
    statusLabel = <span className="session-answer-status">Your choice</span>
  }

  return (
    <button
      type="button"
      className={classNames.join(' ')}
      onClick={() => onSelect(id)}
      disabled={isSubmitted}
      aria-pressed={isSelected}
    >
      <span className="session-answer-label" aria-hidden="true">
        {label}
      </span>
      <span className="session-answer-text">{text}</span>
      {statusLabel}
    </button>
  )
}
