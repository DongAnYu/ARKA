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
      <h3>{isCorrect ? 'Correct' : 'Incorrect'}</h3>
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
