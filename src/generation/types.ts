export type GenerationMode = 'default' | 'graph'

export type GenerationPurpose =
  | 'balanced'
  | 'foundations'
  | 'explanations'
  | 'practical_application'

/** Captured per Default job; the ceiling counts learning targets, not variants. */
export type GenerationOptions = Readonly<{
  max_learning_items: number
  purpose: GenerationPurpose
}>

export type Note = {
  id: number | null
  path: string
  title: string
  content: string
  last_modified: string
}

export type NoteGenerationReport = {
  note_path: string
  note_title: string
  total_chunks: number
}

export type ChunkLlmQuestionPreview = {
  question: string
  option_a: string
  option_b: string
  option_c: string
  option_d: string
  correct_answer: string
  explanation: string
}

export type GeneratedFlashcard = {
  prompt: string
}

export type GeneratedMcq = {
  prompt: string | null
  distractors: string[]
}

export type GeneratedItem = {
  knowledge_point_id: string
  target: string
  answer: string
  explanation: string | null
  flashcard: GeneratedFlashcard
  mcq: GeneratedMcq | null
  mcq_omission_reason: string | null
}

export type GenerationMetadata = {
  model: string | null
  provider: string | null
  pipeline: 'chunk' | 'graph' | null
  generated_at: string | null
}

export type SourceReference = {
  note_path: string
  start_line: number
  end_line: number
  knowledge_point: string
}

export type LearningItemDraft = {
  draft_id: string
  generation: GenerationMetadata
  source: SourceReference
  content: GeneratedItem
}

export type GeneratedLearningItemSaveInput = {
  draft_id: string
  content: GeneratedItem
}

export type ChunkLlmResult = {
  status: string
  key_points: string[]
  items: LearningItemDraft[]
  questions: ChunkLlmQuestionPreview[]
  error: string | null
}

export type ChunkPreview = {
  note_path: string
  note_title: string
  heading: string
  section_index: number
  chunk_index: number
  start_line: number
  end_line: number
  char_count: number
  preview_text: string
  llm_result: ChunkLlmResult
}

export type GenerationSummary = {
  total_notes: number
  total_chunks: number
  notes_with_chunks: number
  note_reports: NoteGenerationReport[]
  chunk_previews: ChunkPreview[]
  default_selection?: DefaultSelectionReport | null
}

export type DefaultSelectionReport = {
  options: GenerationOptions
  policy_version: string
  extracted_chunks: number
  extraction_failed_chunks: number
  candidate_count: number
  assessed_count: number
  unresolved_assessments: number
  unsuitable_count: number
  source_context_reassessments: number
  eligible_count: number
  substantive_sections: number
  selected_sections: number
  selected_count: number
  selection_complete: boolean
  chunk_capacity_excluded: number
  generation_total_chunks: number
  generation_completed_chunks: number
  generation_failed_chunks: number
  generation_failed_targets: number
  generation_omitted_count: number
  generated_count: number
  shortfall: { reason: string; message: string }[]
}

export type LlmFailureCode =
  | 'setup'
  | 'account'
  | 'connection'
  | 'rate_limited'
  | 'provider_unavailable'
  | 'request_rejected'
  | 'invalid_response'
  | 'unknown'

export type LlmFailure = {
  code: LlmFailureCode
  message: string
  retryable: boolean
  retry_after_secs: number | null
}

export type GenerationProgress = {
  default_selection?: DefaultSelectionReport | null
  job_id: string
  total_notes: number
  total_chunks: number
  notes_with_chunks: number
  completed_chunks: number
  mcq_generated: number
  progress_percent: number
  failed_chunks: number
  warnings: LlmFailure[]
  recall_mcq_generated: number
  relational_mcq_generated: number
  current_chunk: number | null
  activity: string | null
  is_paused: boolean
  is_cancelled: boolean
  is_finished: boolean
  error: LlmFailure | null
  ready_previews: ChunkPreview[]
  summary: GenerationSummary | null
  phase_label: string | null
}
