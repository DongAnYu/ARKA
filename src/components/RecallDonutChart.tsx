type ChartCategory = {
  label: string
  value: number
  tone: 'due' | 'overdue' | 'new' | 'correct' | 'muted'
}

export function RecallDonutChart({ label, categories, centerValue, centerLabel }: {
  label: string
  categories: ChartCategory[]
  centerValue: string | number
  centerLabel: string
}) {
  const total = categories.reduce((sum, category) => sum + category.value, 0)
  const description = categories.map((category) => `${category.label}: ${category.value}`).join(', ')

  return (
    <div className="recall-donut-summary">
      <div className="recall-donut" role="img" aria-label={`${label}. ${description}. Total: ${total}.`}>
        <svg viewBox="0 0 120 120" aria-hidden="true">
          <circle className="recall-donut-track" cx="60" cy="60" r="46" pathLength="100" />
          {categories.map((category, index) => {
            if (category.value === 0 || total === 0) return null
            const percentage = category.value / total * 100
            const segmentOffset = categories.slice(0, index).reduce((sum, previous) => sum + previous.value, 0) / total * 100
            return (
              <circle key={category.label} className={`recall-donut-segment is-${category.tone}`}
                cx="60" cy="60" r="46" pathLength="100"
                strokeDasharray={`${percentage} ${100 - percentage}`} strokeDashoffset={-segmentOffset} />
            )
          })}
        </svg>
        <div className="recall-donut-center" aria-hidden="true">
          <strong>{centerValue}</strong>
          <span>{centerLabel}</span>
        </div>
      </div>
      <dl className="recall-donut-key">
        {categories.map((category) => (
          <div key={category.label}>
            <dt><span className={`recall-chart-swatch is-${category.tone}`} aria-hidden="true" />{category.label}</dt>
            <dd>{category.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  )
}
