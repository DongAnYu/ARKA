import { Link } from 'react-router-dom'

type SessionCompleteProps = {
  reviewedCount: number
  correctCount: number
  durationLabel: string
}

export function SessionComplete({ reviewedCount, correctCount, durationLabel }: SessionCompleteProps) {
  return (
    <section className="session-complete surface-panel" aria-label="Session complete">
      <p className="session-complete-mark" aria-hidden="true">
        ✓
      </p>
      <h1>Session Complete</h1>
      <p className="session-complete-summary">{reviewedCount} questions reviewed</p>

      <dl className="session-complete-stats">
        <div>
          <dt>Accuracy</dt>
          <dd>{reviewedCount === 0 ? 0 : Math.round((correctCount / reviewedCount) * 100)}%</dd>
        </div>
        <div>
          <dt>Duration</dt>
          <dd>{durationLabel}</dd>
        </div>
      </dl>

      <Link to="/questions" className="btn-primary session-complete-link">
        Return to Recall Space
      </Link>
    </section>
  )
}
