import { SessionProgress } from './SessionProgress'

type SessionHeaderProps = {
  sessionLabel: string
  recallSpaceName: string
  currentItemNumber: number
  totalItems: number
  studiedCount?: number
}

export function SessionHeader({
  sessionLabel,
  recallSpaceName,
  currentItemNumber,
  totalItems,
  studiedCount,
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
          {studiedCount === undefined ? `Item ${currentItemNumber} of ${totalItems}`
            : `${studiedCount} ${studiedCount === 1 ? 'item' : 'items'} studied this session`}
        </p>
        {studiedCount === undefined ? <SessionProgress value={progressPercent} /> : null}
      </div>
    </header>
  )
}
