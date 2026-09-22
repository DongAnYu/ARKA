import { Check } from 'lucide-react'
import { useEffect, useRef } from 'react'

type SessionCompleteProps = {
  reviewedCount: number
  recalledCount: number
  mode: 'recall' | 'learn-new'
  onReturn: () => void
}

type ReviewPieChartProps = {
  reviewedCount: number
  recalledCount: number
  againCount: number
  recallPercent: number
}

function ReviewPieChart({
  reviewedCount,
  recalledCount,
  againCount,
  recallPercent,
}: ReviewPieChartProps) {
  const categories = [
    { label: 'Recalled', value: recalledCount, className: 'is-correct' },
    { label: 'Again', value: againCount, className: 'is-incorrect' },
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
        <strong>{recallPercent}%</strong>
        <span>Recalled</span>
      </div>
    </div>
  )
}

export function SessionComplete({
  reviewedCount,
  recalledCount,
  mode,
  onReturn,
}: SessionCompleteProps) {
  const heading = useRef<HTMLHeadingElement>(null)
  useEffect(() => { heading.current?.focus() }, [])
  const normalizedReviewedCount = Math.max(0, reviewedCount)
  const normalizedRecalledCount = Math.min(Math.max(0, recalledCount), normalizedReviewedCount)
  const againCount = normalizedReviewedCount - normalizedRecalledCount
  const recallPercent =
    normalizedReviewedCount === 0 ? 0 : Math.round((normalizedRecalledCount / normalizedReviewedCount) * 100)
  const isLearnNew = mode === 'learn-new'

  return (
    <section className="session-complete surface-panel" aria-label="Session complete">
      <span className="session-complete-mark" aria-hidden="true">
        <Check />
      </span>
      <h1 ref={heading} tabIndex={-1}>{isLearnNew ? 'Learning Complete' : 'Session Complete'}</h1>
      <p className="session-complete-summary">
        {isLearnNew
          ? `${normalizedReviewedCount} new ${normalizedReviewedCount === 1 ? 'item is' : 'items are'} now scheduled for future recall.`
          : 'Here\'s how this recall session went.'}
      </p>

      <div className="session-complete-dashboard">
        <ReviewPieChart
          reviewedCount={normalizedReviewedCount}
          recalledCount={normalizedRecalledCount}
          againCount={againCount}
          recallPercent={recallPercent}
        />

        <dl className="session-complete-metrics">
          <div>
            <dt>Reviewed</dt>
            <dd>{normalizedReviewedCount}</dd>
          </div>
          <div>
            <dt>
              <span className="session-complete-swatch is-correct" aria-hidden="true" />
              Recalled
            </dt>
            <dd>{normalizedRecalledCount}</dd>
          </div>
          <div>
            <dt>
              <span className="session-complete-swatch is-incorrect" aria-hidden="true" />
              Again
            </dt>
            <dd>{againCount}</dd>
          </div>
        </dl>
      </div>

      <button type="button" className="btn-primary session-complete-link" onClick={onReturn}>
        Return to Recall
      </button>
    </section>
  )
}
