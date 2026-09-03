export type GenerationMode = 'default' | 'graph'

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

export type ChunkLlmResult = {
  status: string
  key_points: string[]
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
  summary: GenerationSummary | null
  phase_label: string | null
}
