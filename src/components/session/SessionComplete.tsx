import { Check } from 'lucide-react'

type SessionCompleteProps = {
  reviewedCount: number
  correctCount: number
  onReturn: () => void
}

type ReviewPieChartProps = {
  reviewedCount: number
  correctCount: number
  incorrectCount: number
  accuracy: number
}

function ReviewPieChart({
  reviewedCount,
  correctCount,
  incorrectCount,
  accuracy,
}: ReviewPieChartProps) {
  const categories = [
    { label: 'Correct', value: correctCount, className: 'is-correct' },
    { label: 'Incorrect', value: incorrectCount, className: 'is-incorrect' },
  ]
  let offset = 0

  return (
    <div className="session-results-pie" aria-hidden="true">
      <svg viewBox="0 0 120 120" aria-hidden="true">
        <circle className="session-results-pie-track" cx="60" cy="60" r="46" pathLength="100" />
        {categories.map((category) => {
          const percentage = reviewedCount === 0 ? 0 : (category.value / reviewedCount) * 100
          const segmentOffset = offset
          offset += percentage

          if (category.value === 0) {
            return null
          }

          return (
            <circle
              key={category.label}
              className={`session-results-pie-segment ${category.className}`}
              cx="60"
              cy="60"
              r="46"
              pathLength="100"
              strokeDasharray={`${percentage} ${100 - percentage}`}
              strokeDashoffset={-segmentOffset}
            />
          )
        })}
      </svg>
      <div className="session-results-pie-center" aria-hidden="true">
        <strong>{accuracy}%</strong>
        <span>Accuracy</span>
      </div>
    </div>
  )
}

export function SessionComplete({
  reviewedCount,
  correctCount,
  onReturn,
}: SessionCompleteProps) {
  const normalizedReviewedCount = Math.max(0, reviewedCount)
  const normalizedCorrectCount = Math.min(Math.max(0, correctCount), normalizedReviewedCount)
  const incorrectCount = normalizedReviewedCount - normalizedCorrectCount
  const accuracy =
    normalizedReviewedCount === 0 ? 0 : Math.round((normalizedCorrectCount / normalizedReviewedCount) * 100)

  return (
    <section className="session-complete surface-panel" aria-label="Session complete">
      <span className="session-complete-mark" aria-hidden="true">
        <Check />
      </span>
      <h1>Session Complete</h1>
      <p className="session-complete-summary">Here&apos;s how this recall session went.</p>

      <div className="session-complete-dashboard">
        <ReviewPieChart
          reviewedCount={normalizedReviewedCount}
          correctCount={normalizedCorrectCount}
          incorrectCount={incorrectCount}
          accuracy={accuracy}
        />

        <dl className="session-complete-metrics">
          <div>
            <dt>Accuracy</dt>
            <dd>{accuracy}%</dd>
          </div>
          <div>
            <dt>Questions reviewed</dt>
            <dd>{normalizedReviewedCount}</dd>
          </div>
          <div>
            <dt>
              <span className="session-complete-swatch is-correct" aria-hidden="true" />
              Correct
            </dt>
            <dd>{normalizedCorrectCount}</dd>
          </div>
          <div>
            <dt>
              <span className="session-complete-swatch is-incorrect" aria-hidden="true" />
              Incorrect
            </dt>
            <dd>{incorrectCount}</dd>
          </div>
        </dl>
      </div>

      <button type="button" className="btn-primary session-complete-link" onClick={onReturn}>
        Return to Recall
      </button>
    </section>
  )
}
