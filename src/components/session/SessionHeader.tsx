import { SessionProgress } from './SessionProgress'

type SessionHeaderProps = {
  sessionLabel: string
  recallSpaceName: string
  currentItemNumber: number
  totalItems: number
}

export function SessionHeader({
  sessionLabel,
  recallSpaceName,
  currentItemNumber,
  totalItems,
}: SessionHeaderProps) {
  const progressPercent = totalItems === 0 ? 0 : (currentItemNumber / totalItems) * 100

  return (
    <header className="session-header surface-panel" aria-label="Session header">
      <div className="session-header-meta">
        <p className="session-space-label">{sessionLabel}</p>
        <h1>{recallSpaceName}</h1>
      </div>

      <div className="session-progress-meta" aria-live="polite">
        <p>
          Item {currentItemNumber} of {totalItems}
        </p>
        <SessionProgress value={progressPercent} />
      </div>
    </header>
  )
}
