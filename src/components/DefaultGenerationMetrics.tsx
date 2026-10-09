import { LEARNING_PURPOSES } from '../generation/options'
import type { DefaultSelectionReport, GenerationOptions } from '../generation/types'

export function DefaultGenerationMetrics({ report, options, finished = false }: {
  report?: DefaultSelectionReport | null
  options: GenerationOptions
  finished?: boolean
}) {
  const capturedOptions = report?.options ?? options
  return (
    <div className="generation-selection-metrics">
      <dl className="generation-selection-settings">
        <div><dt>Requested maximum</dt><dd>{capturedOptions.max_learning_items}</dd></div>
        <div><dt>Learning purpose</dt><dd>{LEARNING_PURPOSES[capturedOptions.purpose].label}</dd></div>
      </dl>
      <dl className="generation-selection-results">
        <div><dt>Concepts assessed</dt><dd>{report?.assessed_count ?? 0} / {report?.candidate_count ?? '—'}</dd></div>
        <div><dt>Concepts selected</dt><dd>{report?.selection_complete ? report.selected_count : 'Pending'}</dd></div>
        {finished && <div><dt>Learning items generated</dt><dd>{report?.generated_count ?? 0}</dd></div>}
        {report?.selection_complete && (
          <div><dt>Sections represented</dt><dd>{report.selected_sections} / {report.substantive_sections}</dd></div>
        )}
      </dl>
      {!!report?.unresolved_assessments && !finished && (
        <p className="generation-selection-warning" role="status">
          {report.unresolved_assessments} {report.unresolved_assessments === 1 ? 'concept' : 'concepts'} excluded after failed or unresolved assessment.
        </p>
      )}
      {finished && !!report?.shortfall.length && (
        <div className="generation-shortfall" role="status">
          <strong>{report.generated_count} of up to {capturedOptions.max_learning_items} learning items generated</strong>
          <ul>{report.shortfall.map((detail) => <li key={detail.reason}>{detail.message}</li>)}</ul>
          <p>The maximum is a ceiling. Selected concepts and generated items can differ.</p>
        </div>
      )}
    </div>
  )
}
