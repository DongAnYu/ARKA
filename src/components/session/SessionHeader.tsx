import { SessionProgress } from './SessionProgress'

type SessionHeaderProps = {
  recallSpaceName: string
  currentQuestionNumber: number
  totalQuestions: number
}

export function SessionHeader({
  recallSpaceName,
  currentQuestionNumber,
  totalQuestions,
}: SessionHeaderProps) {
  const progressPercent = totalQuestions === 0 ? 0 : (currentQuestionNumber / totalQuestions) * 100

  return (
    <header className="session-header surface-panel" aria-label="Session header">
      <div className="session-header-meta">
        <p className="session-space-label">Recall Space</p>
        <h1>{recallSpaceName}</h1>
      </div>

      <div className="session-progress-meta" aria-live="polite">
        <p>
          Question {currentQuestionNumber} of {totalQuestions}
        </p>
        <SessionProgress value={progressPercent} />
      </div>
    </header>
  )
}
