import { Check, X } from 'lucide-react'

type ExplanationPanelProps = {
  isCorrect: boolean
  selectedAnswer: string
  correctAnswer: string
  explanation: string | null
}

export function ExplanationPanel({
  isCorrect,
  selectedAnswer,
  correctAnswer,
  explanation,
}: ExplanationPanelProps) {
  return (
    <section
      className={`session-explanation surface-panel${isCorrect ? ' is-correct' : ' is-incorrect'}`}
      aria-live="polite"
    >
      <div className="session-explanation-heading">
        <span className="session-explanation-symbol" aria-hidden="true">
          {isCorrect ? <Check /> : <X />}
        </span>
        <h3>{isCorrect ? 'Correct' : 'Incorrect'}</h3>
      </div>
      <p>
        Correct answer: <strong>{correctAnswer}</strong>
      </p>
      {!isCorrect ? (
        <p>
          Your answer: <strong>{selectedAnswer}</strong>
        </p>
      ) : null}
      {explanation ? <p>{explanation}</p> : null}
    </section>
  )
}
