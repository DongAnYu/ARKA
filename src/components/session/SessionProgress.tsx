type SessionProgressProps = {
  value: number
}

export function SessionProgress({ value }: SessionProgressProps) {
  const clampedValue = Math.min(100, Math.max(0, value))

  return (
    <div className="session-progress" aria-hidden="true">
      <div className="session-progress-fill" style={{ width: `${clampedValue}%` }} />
    </div>
  )
}
