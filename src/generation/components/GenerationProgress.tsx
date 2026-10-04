import { Check } from 'lucide-react'

type ProgressRingProps = {
  percent: number
  label: string
  caption?: string
  className?: string
}

export function GenerationProgressRing({ percent, label, caption = 'complete', className = '' }: ProgressRingProps) {
  return (
    <div
      className={`generation-progress-ring ${className}`.trim()}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
    >
      <svg viewBox="0 0 176 176" aria-hidden="true">
        <circle className="generation-ring-track" cx="88" cy="88" r="72" />
        <circle
          className="generation-ring-value"
          cx="88"
          cy="88"
          r="72"
          pathLength="1"
          style={{ strokeDashoffset: 1 - percent / 100 }}
        />
      </svg>
      <div className="generation-ring-label">
        <strong>{percent}%</strong>
        <span>{caption}</span>
      </div>
    </div>
  )
}

type GenerationPhasesProps = {
  phases: readonly string[]
  currentIndex: number
  paused: boolean
  label: string
}

export function GenerationPhases({ phases, currentIndex, paused, label }: GenerationPhasesProps) {
  return (
    <ol className="generation-phase-list" aria-label={label}>
      {phases.map((phase, index) => {
        const complete = index < currentIndex
        const current = index === currentIndex
        let status = 'Waiting'
        if (complete) status = 'Complete'
        if (current) status = paused ? 'Paused' : 'In progress'

        return (
          <li key={phase} className={complete ? 'is-complete' : current ? 'is-current' : ''} aria-current={current ? 'step' : undefined}>
            <span aria-hidden="true">{complete ? <Check /> : index + 1}</span>
            <div>
              <strong>{phase}</strong>
              <small>{status}</small>
            </div>
          </li>
        )
      })}
    </ol>
  )
}
