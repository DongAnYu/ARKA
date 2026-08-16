import { Check, X } from 'lucide-react'

type ExplanationPanelProps = {
  isCorrect: boolean
  selectedOptionId: string
  correctOptionId: string
  explanation: string | null
}

export function ExplanationPanel({
  isCorrect,
  selectedOptionId,
  correctOptionId,
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
        Correct answer: <strong>{correctOptionId}</strong>
      </p>
      {!isCorrect ? (
        <p>
          Your answer: <strong>{selectedOptionId}</strong>
        </p>
      ) : null}
      <p>{explanation ?? 'No explanation was provided for this question yet.'}</p>
    </section>
  )
}
