import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import {
  AlertTriangle,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronRight,
  FileText,
  FolderOpen,
  GitBranch,
  Pause,
  Play,
  Plus,
  Save,
  Sparkles,
  X,
  Zap,
} from 'lucide-react'
import arkaLogo from '../assets/arka-logo.svg'

const getWelcomeMessage = (date = new Date()) => {
  const hour = date.getHours()

  if (hour >= 5 && hour < 12) {
    return (
      <>
        Good morning!
        <br />
        Ready to test your recall?
      </>
    )
  }

  if (hour >= 12 && hour < 17) {
    return (
      <>
        Good afternoon!
        <br />
        Ready to test your recall?
      </>
    )
  }

  if (hour >= 17 && hour < 22) {
    return (
      <>
        Good evening!
        <br />
        Ready to test your recall?
      </>
    )
  }

  return (
    <>
      Late night, 
      <br />
      Ready to test your recall?
    </>
  )
}
type GenerationMode = 'default' | 'graph'

type Note = {
  id: number | null
  path: string
  title: string
  content: string
  last_modified: string
}

type NoteGenerationReport = {
  note_path: string
  note_title: string
  total_chunks: number
}

type ChunkLlmQuestionPreview = {
  question: string
  option_a: string
  option_b: string
  option_c: string
  option_d: string
  correct_answer: string
  explanation: string
}

type ChunkLlmResult = {
  status: string
  key_points: string[]
  questions: ChunkLlmQuestionPreview[]
  error: string | null
}

type ChunkPreview = {
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

type GenerationSummary = {
  total_notes: number
  total_chunks: number
  notes_with_chunks: number
  note_reports: NoteGenerationReport[]
  chunk_previews: ChunkPreview[]
}

type LlmFailureCode =
  | 'setup'
  | 'account'
  | 'connection'
  | 'rate_limited'
  | 'provider_unavailable'
  | 'request_rejected'
  | 'invalid_response'
  | 'unknown'

type LlmFailure = {
  code: LlmFailureCode
  message: string
  retryable: boolean
  retry_after_secs: number | null
}

type GenerationProgress = {
  job_id: string
  total_notes: number
  total_chunks: number
  notes_with_chunks: number
  completed_chunks: number
  mcq_generated: number
  progress_percent: number
  failed_chunks: number
  warnings: LlmFailure[]
  current_chunk: number | null
  activity: string | null
  is_paused: boolean
  is_cancelled: boolean
  is_finished: boolean
  error: LlmFailure | null
  summary: GenerationSummary | null
  phase_label: string | null
}

type Question = {
  question: string
  option_a: string
  option_b: string
  option_c: string
  option_d: string
  correct_answer: string
  explanation: string | null
  space_id: number
}

type RecallSpace = {
  id: number
  name: string
  description: string | null
}

type ModelConfig = {
  selected_model: string
}

export function HomePage() {
  const welcomeMessage = getWelcomeMessage()
  const saveInFlightRef = useRef(false)
  const [vaultPath, setVaultPath] = useState('')
  const [notes, setNotes] = useState<Note[]>([])
  const [selectedNote, setSelectedNote] = useState<Note | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isGenerating, setIsGenerating] = useState(false)
  const [isSavingQuestions, setIsSavingQuestions] = useState(false)
  const [generationJobId, setGenerationJobId] = useState<string | null>(null)
  const [generationProgress, setGenerationProgress] = useState<GenerationProgress | null>(null)
  const [generationSummary, setGenerationSummary] = useState<GenerationSummary | null>(null)
  const [showChunks, setShowChunks] = useState(false)
  const [selectedChunk, setSelectedChunk] = useState<ChunkPreview | null>(null)
  const [isLoadingRecallSpaces, setIsLoadingRecallSpaces] = useState(true)
  const [recallSpaces, setRecallSpaces] = useState<RecallSpace[]>([])
  const [selectedSpaceId, setSelectedSpaceId] = useState(1)
  const [saveDestinationMode, setSaveDestinationMode] = useState<'existing' | 'new'>('existing')
  const [newSpaceName, setNewSpaceName] = useState('')
  const [newSpaceDescription, setNewSpaceDescription] = useState('')
  const [saveStatus, setSaveStatus] = useState('')
  const [saveStatusKind, setSaveStatusKind] = useState<'success' | 'error' | ''>('')
  const [hasSavedQuestions, setHasSavedQuestions] = useState(false)
  const [error, setError] = useState('')
  const [generationMode, setGenerationMode] = useState<GenerationMode | null>(null)

  const generatedQuestionCount =
    generationSummary?.chunk_previews.reduce(
      (total, chunk) => total + chunk.llm_result.questions.length,
      0,
    ) ?? 0

  const selectedSpace = recallSpaces.find((space) => space.id === selectedSpaceId)

  const noteBreadcrumbs = selectedNote
    ? selectedNote.path
        .split(/[\\/]+/)
        .slice(0, -1)
        .filter((part) => part && !/^[A-Za-z]:$/.test(part))
        .slice(-3)
    : []

  const loadRecallSpaces = async () => {
    try {
      const spaces = await invoke<RecallSpace[]>('get_spaces')
      setRecallSpaces(spaces)

      if (spaces.length > 0 && !spaces.some((space) => space.id === selectedSpaceId)) {
        setSelectedSpaceId(spaces[0].id)
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load recall spaces'
      setError(message)
    }
  }

  useEffect(() => {
    let cancelled = false

    void invoke<RecallSpace[]>('get_spaces')
      .then((spaces) => {
        if (cancelled) {
          return
        }

        setRecallSpaces(spaces)
        if (spaces.length > 0) {
          setSelectedSpaceId((currentId) =>
            spaces.some((space) => space.id === currentId) ? currentId : spaces[0].id,
          )
        }
      })
      .catch((err) => {
        if (cancelled) {
          return
        }

        const message = err instanceof Error ? err.message : 'Failed to load recall spaces'
        setError(message)
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoadingRecallSpaces(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [])

  const chooseVault = async () => {
    setError('')

    try {
      const selected = await open({
        directory: false,
        multiple: false,
        title: 'Choose Markdown File',
        filters: [
          {
            name: 'Markdown',
            extensions: ['md', 'markdown'],
          },
        ],
      })

      if (typeof selected !== 'string') {
        return
      }

      setVaultPath(selected)
      await loadNotes(selected)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to choose markdown file'
      setError(message)
    }
  }

  const loadNotes = async (path: string) => {
    setIsLoading(true)
    setError('')
    setSelectedNote(null)
    setGenerationSummary(null)
    setShowChunks(false)
    setSelectedChunk(null)
    setSaveStatus('')
    setSaveStatusKind('')
    setHasSavedQuestions(false)
    setGenerationMode(null)

    try {
      const data = await invoke<Note[]>('get_notes', { vaultPath: path })
      setNotes(data)
      if (data.length === 1) {
        setSelectedNote(data[0])
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load notes'
      setError(message)
    } finally {
      setIsLoading(false)
    }
  }

  const generatePreview = async () => {
    if (!vaultPath || !selectedNote) {
      setError('Choose a note before generating questions.')
      return
    }

    if (!generationMode) {
      setError('Choose a generation mode first.')
      return
    }

    setIsGenerating(true)
    setError('')
    setGenerationProgress(null)
    setGenerationSummary(null)
    setShowChunks(false)
    setSelectedChunk(null)

    const command = generationMode === 'graph' ? 'start_graph_generation_job' : 'start_preview_generation'
    try {
      const jobId = await invoke<string>(command, {
        vaultPath,
      })
      setGenerationJobId(jobId)
    } catch (err) {
      const message =
        typeof err === 'string'
          ? err
          : err instanceof Error
            ? err.message
            : 'Failed to start question generation'
      setError(message)
      setIsGenerating(false)
    }
  }

  useEffect(() => {
    if (!generationJobId) {
      return
    }

    let disposed = false

    const poll = async () => {
      try {
        const progress = await invoke<GenerationProgress>('get_preview_generation_progress', {
          jobId: generationJobId,
        })

        if (disposed) {
          return
        }

        setGenerationProgress(progress)
        setIsGenerating(!progress.is_finished)

        if (progress.is_finished) {
          if (progress.summary) {
            setGenerationSummary(progress.summary)
          }
          setGenerationJobId(null)
        }
      } catch (err) {
        if (!disposed) {
          const message = err instanceof Error ? err.message : 'Failed to load generation progress'
          setError(message)
          setIsGenerating(false)
          setGenerationJobId(null)
        }
      }
    }

    poll()
    const timer = window.setInterval(poll, 700)

    return () => {
      disposed = true
      window.clearInterval(timer)
    }
  }, [generationJobId])

  const togglePauseGeneration = async () => {
    if (!generationProgress) {
      return
    }

    const nextPaused = !generationProgress.is_paused
    try {
      await invoke('set_preview_generation_paused', {
        jobId: generationProgress.job_id,
        paused: nextPaused,
      })
      setGenerationProgress((current) =>
        current ? { ...current, is_paused: nextPaused } : current,
      )
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to toggle generation pause'
      setError(message)
    }
  }

  const cancelGeneration = async () => {
    if (!generationProgress) {
      return
    }

    try {
      await invoke('cancel_preview_generation', {
        jobId: generationProgress.job_id,
      })
      setGenerationProgress(null)
      setGenerationJobId(null)
      setIsGenerating(false)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to cancel generation'
      setError(message)
    }
  }

  const saveGeneratedQuestions = async () => {
    if (saveInFlightRef.current || isSavingQuestions || hasSavedQuestions) {
      return
    }

    if (!generationSummary) {
      setSaveStatus('No questions to save.')
      setSaveStatusKind('error')
      return
    }

    if (generatedQuestionCount === 0) {
      setSaveStatus('No generated questions were found in this preview.')
      setSaveStatusKind('error')
      return
    }

    if (saveDestinationMode === 'new' && !newSpaceName.trim()) {
      setSaveStatus('Name the new recall space before saving.')
      setSaveStatusKind('error')
      return
    }

    if (saveDestinationMode === 'existing' && !selectedSpaceId) {
      setSaveStatus('Choose a recall space before saving.')
      setSaveStatusKind('error')
      return
    }

    saveInFlightRef.current = true
    setIsSavingQuestions(true)
    setSaveStatus('')
    setSaveStatusKind('')

    try {
      let destinationSpaceId = selectedSpaceId

      if (saveDestinationMode === 'new') {
        const trimmedName = newSpaceName.trim()

        const createdSpace = await invoke<RecallSpace>('create_space', {
          name: trimmedName,
          description: newSpaceDescription.trim() || null,
        })

        destinationSpaceId = createdSpace.id
        setSelectedSpaceId(createdSpace.id)
        setSaveDestinationMode('existing')
        setNewSpaceName('')
        setNewSpaceDescription('')
        await loadRecallSpaces()
      }

      // Extract all questions from chunks
      const questionsToSave: Question[] = []
      generationSummary.chunk_previews.forEach((chunk) => {
        chunk.llm_result.questions.forEach((q) => {
          questionsToSave.push({
            question: q.question,
            option_a: q.option_a,
            option_b: q.option_b,
            option_c: q.option_c,
            option_d: q.option_d,
            correct_answer: q.correct_answer,
            explanation: q.explanation,
            space_id: destinationSpaceId,
          })
        })
      })

      // Load current model config to get model name
      const modelConfig = await invoke<ModelConfig>('load_model_config')
      const modelName = modelConfig.selected_model || 'unknown'

      // Save to database
      await invoke('save_generated_questions', {
        questions: questionsToSave,
        model: modelName,
      })

      // Success feedback
      const spaceName =
        saveDestinationMode === 'new'
          ? newSpaceName.trim()
          : selectedSpace?.name ?? `space ${destinationSpaceId}`
      setHasSavedQuestions(true)
      setSaveStatus(`Saved ${questionsToSave.length} questions to ${spaceName}.`)
      setSaveStatusKind('success')
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save questions'
      setSaveStatus(message)
      setSaveStatusKind('error')
      setHasSavedQuestions(false)
    } finally {
      saveInFlightRef.current = false
      setIsSavingQuestions(false)
    }
  }

  return (
    <div className={`app-container home-page${selectedNote ? ' has-selected-note' : ''}`}>
      {!selectedNote ? (
        <>
          <div className="app-header">
            <div className="brand-title">
              <img src={arkaLogo} alt="ARKA logo" className="brand-logo" />
              <h1>{welcomeMessage}</h1>
            </div>
            <p className="welcome-description">
              Recall reads your notes and generates thoughtful flashcards or
              multiple-choice questions, so you can turn what you&apos;ve written into
              what you actually remember.
            </p>
            <div className="header-actions">
              <button
                type="button"
                className="btn-primary"
                onClick={chooseVault}
                disabled={isLoading}
              >
                {isLoading ? (
                  'Loading note…'
                ) : (
                  <span className="btn-content">
                    Choose Markdown File
                    <ArrowRight className="size-4" aria-hidden="true" />
                  </span>
                )}
              </button>
            </div>
          </div>

          {error && <p className="error-text" role="alert">{error}</p>}

          <div className="notes-container" aria-live="polite">
            {notes.length > 1 && (
              <>
                <p className="notes-count">Choose one of {notes.length} notes</p>
                <ul className="note-list">
                  {notes.map((note) => (
                    <li key={note.path}>
                      <button
                        type="button"
                        className="note-item"
                        onClick={() => {
                          setGenerationMode(null)
                          setSelectedNote(note)
                        }}
                      >
                        {note.title}
                      </button>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>
        </>
      ) : (
        <main className="generation-workspace">
          <header className="selected-note-header">
            {noteBreadcrumbs.length > 0 && (
              <nav className="note-breadcrumbs" aria-label="Note location">
                {noteBreadcrumbs.map((part, index) => (
                  <span key={`${part}-${index}`}>
                    {index > 0 && <ChevronRight aria-hidden="true" />}
                    <span>{part}</span>
                  </span>
                ))}
              </nav>
            )}

            <div className="selected-note-row">
              <div className="selected-note-title-wrap">
                <FileText className="selected-note-icon" aria-hidden="true" />
                <div>
                  <h1>{selectedNote.title}</h1>
                  <p>Ready to turn this note into active recall.</p>
                </div>
              </div>
              <button
                type="button"
                className="btn-secondary btn-change-note"
                onClick={chooseVault}
                disabled={isLoading || isGenerating}
              >
                <FolderOpen className="size-4" aria-hidden="true" />
                {isLoading ? 'Loading…' : 'Change note'}
              </button>
            </div>
          </header>

          {error && <p className="error-text" role="alert">{error}</p>}

          {!isGenerating && !generationSummary && (
            <section className="generation-setup" aria-labelledby="generation-depth-title">
              <div className="generation-setup-heading">
                <h2 id="generation-depth-title">Choose how deep to go</h2>
                <p>Pick the reasoning approach that fits this study session.</p>
              </div>

              <div className="generation-mode-grid" role="radiogroup" aria-label="Generation mode">
                <button
                  type="button"
                  role="radio"
                  aria-checked={generationMode === 'default'}
                  className={`generation-mode-card${generationMode === 'default' ? ' is-selected' : ''}`}
                  onClick={() => {
                    setError('')
                    setGenerationMode('default')
                  }}
                >
                  <span className="generation-mode-icon">
                    <Zap aria-hidden="true" />
                  </span>
                  <span className="generation-mode-copy">
                    <strong>Default generation</strong>
                    <span>Light reasoning. Extracts focused questions straight from the note.</span>
                  </span>
                  <span className="generation-mode-meta">Faster</span>
                  {generationMode === 'default' && (
                    <span className="generation-mode-check" aria-hidden="true">
                      <Check />
                    </span>
                  )}
                </button>

                <button
                  type="button"
                  role="radio"
                  aria-checked={generationMode === 'graph'}
                  className={`generation-mode-card${generationMode === 'graph' ? ' is-selected' : ''}`}
                  onClick={() => {
                    setError('')
                    setGenerationMode('graph')
                  }}
                >
                  <span className="generation-mode-icon">
                    <GitBranch aria-hidden="true" />
                  </span>
                  <span className="generation-mode-copy">
                    <strong>Deep thinking</strong>
                    <span>Builds a knowledge graph to generate more connected, reasoning-focused questions.</span>
                  </span>
                  <span className="generation-mode-meta">More thorough</span>
                  {generationMode === 'graph' && (
                    <span className="generation-mode-check" aria-hidden="true">
                      <Check />
                    </span>
                  )}
                </button>
              </div>

              {generationMode && (
                <div className="generation-commit">
                  <p>
                    <strong>{generationMode === 'graph' ? 'Deep thinking' : 'Default generation'}</strong>
                    <span>{generationMode === 'graph' ? 'Best for connected, concept-heavy notes.' : 'Best for a quick, focused study set.'}</span>
                  </p>
                  <button
                    type="button"
                    className="btn-primary btn-start-generation"
                    onClick={generatePreview}
                  >
                    <span className="btn-content">
                      <Sparkles className="size-4" aria-hidden="true" />
                      Generate questions
                    </span>
                  </button>
                </div>
              )}
            </section>
          )}

          {isGenerating && (
            <section className="generation-progress generation-progress-focused" aria-live="polite">
              <div className="generation-progress-intro">
                <span className="generation-progress-symbol" aria-hidden="true">
                  {generationMode === 'graph' ? <GitBranch /> : <Sparkles />}
                </span>
                <div>
                  <h2>{generationProgress?.is_paused ? 'Generation paused' : 'Building your questions'}</h2>
                  <p>
                    {generationProgress?.is_paused
                      ? generationProgress.activity
                        ? `Paused · ${generationProgress.activity}`
                        : 'Generation is paused'
                      : generationProgress?.activity ??
                        (generationProgress?.phase_label
                          ? `${generationProgress.phase_label}…`
                          : 'Preparing your note…')}
                  </p>
                </div>
                <strong>{generationProgress?.progress_percent ?? 0}%</strong>
              </div>

              <div
                className="generation-bar-track"
                role="progressbar"
                aria-label="Question generation progress"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={generationProgress?.progress_percent ?? 0}
              >
                <div
                  className="generation-bar-fill"
                  style={{
                    transform: `scaleX(${(generationProgress?.progress_percent ?? 0) / 100})`,
                  }}
                />
              </div>

              <dl className="generation-progress-stats">
                <div>
                  <dt>{generationMode === 'graph' ? 'Knowledge processed' : 'Chunks completed'}</dt>
                  <dd>{generationProgress ? `${generationProgress.completed_chunks} / ${generationProgress.total_chunks}` : '—'}</dd>
                </div>
                <div>
                  <dt>Questions generated</dt>
                  <dd>{generationProgress?.mcq_generated ?? 0}</dd>
                </div>
                <div>
                  <dt>Skipped chunks</dt>
                  <dd>{generationProgress?.failed_chunks ?? 0}</dd>
                </div>
              </dl>

              {(generationProgress?.failed_chunks ?? 0) > 0 && (
                <p className="generation-inline-warning" role="status">
                  <AlertTriangle aria-hidden="true" />
                  {generationProgress?.failed_chunks === 1
                    ? '1 chunk could not be generated and was skipped. Continuing with the rest.'
                    : `${generationProgress?.failed_chunks} chunks could not be generated and were skipped. Continuing with the rest.`}
                </p>
              )}

              <div className="generation-actions">
                <button
                  type="button"
                  className="btn-pause"
                  onClick={togglePauseGeneration}
                  disabled={!generationProgress}
                >
                  <span
                    className="generation-control-icon"
                    data-state={generationProgress?.is_paused ? 'paused' : 'running'}
                    aria-hidden="true"
                  >
                    <Pause data-icon="running" />
                    <Play data-icon="paused" />
                  </span>
                  <span>{generationProgress?.is_paused ? 'Resume' : 'Pause'}</span>
                </button>
                <button
                  type="button"
                  className="btn-cancel"
                  onClick={cancelGeneration}
                  disabled={!generationProgress}
                >
                  <X aria-hidden="true" />
                  Cancel
                </button>
              </div>
            </section>
          )}

      {generationSummary && (
        <section className="generation-summary" aria-live="polite">
          <div className="generation-summary-head">
            <h2>Questions ready for review</h2>
            <div className="generation-summary-head-actions">
              {generationProgress?.error && (
                <button type="button" className="view-chunks-btn" onClick={generatePreview}>
                  Try again
                </button>
              )}
              <button
                type="button"
                className="view-chunks-btn"
                onClick={() => setShowChunks((prev) => !prev)}
                aria-expanded={showChunks}
              >
                <svg
                  className="view-chunks-icon"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" />
                  <circle cx="12" cy="12" r="3" />
                </svg>
                {showChunks ? 'Hide chunks' : 'View chunks'}
              </button>
            </div>
          </div>

          {generationProgress?.error ? (
            <div className="generation-result-notice is-error" role="alert">
              <AlertTriangle aria-hidden="true" />
              <div>
                <strong>Generation stopped early</strong>
                <p>
                  {generationProgress.error.message}
                  {generatedQuestionCount > 0 &&
                    ` ${generatedQuestionCount} ${generatedQuestionCount === 1 ? 'question is' : 'questions are'} still ready to review and save.`}
                </p>
              </div>
            </div>
          ) : (generationProgress?.failed_chunks ?? 0) > 0 ? (
            <div className="generation-result-notice is-warning" role="status">
              <AlertTriangle aria-hidden="true" />
              <div>
                <strong>
                  Completed with {generationProgress?.failed_chunks}{' '}
                  {generationProgress?.failed_chunks === 1 ? 'skipped chunk' : 'skipped chunks'}
                </strong>
                <p>Questions from the remaining chunks are ready to review and save.</p>
              </div>
            </div>
          ) : null}
          <div className="summary-grid">
            <p>
              <span>Total Notes</span>
              <strong>{generationSummary.total_notes}</strong>
            </p>
            <p>
              <span>Notes With Chunks</span>
              <strong>{generationSummary.notes_with_chunks}</strong>
            </p>
            <p>
              <span>Total Chunks</span>
              <strong>{generationSummary.total_chunks}</strong>
            </p>
          </div>

          <div className="generation-summary-actions">
            <div className="recall-save-panel">
              <div className="recall-save-head">
                <div>
                  <h3>Save to Recall Space</h3>
                  <p>{generatedQuestionCount} questions ready to save</p>
                </div>
                <div className="save-mode-toggle" role="group" aria-label="Save destination">
                  <button
                    type="button"
                    className={`save-mode-btn${saveDestinationMode === 'existing' ? ' is-active' : ''}`}
                    onClick={() => setSaveDestinationMode('existing')}
                    disabled={isSavingQuestions || hasSavedQuestions}
                  >
                    Existing
                  </button>
                  <button
                    type="button"
                    className={`save-mode-btn${saveDestinationMode === 'new' ? ' is-active' : ''}`}
                    onClick={() => setSaveDestinationMode('new')}
                    disabled={isSavingQuestions || hasSavedQuestions}
                  >
                    New
                  </button>
                </div>
              </div>

              {saveDestinationMode === 'existing' ? (
                <label className="recall-space-select-wrap">
                  <span>{isLoadingRecallSpaces ? 'Loading recall spaces…' : 'Recall space'}</span>
                  <select
                    className="recall-space-select"
                    value={selectedSpaceId}
                    onChange={(event) => setSelectedSpaceId(Number(event.target.value))}
                    disabled={
                      isLoadingRecallSpaces ||
                      isSavingQuestions ||
                      hasSavedQuestions ||
                      recallSpaces.length === 0
                    }
                  >
                    {recallSpaces.map((space) => (
                      <option key={space.id} value={space.id}>
                        {space.name}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="recall-space-chevron" aria-hidden="true" />
                </label>
              ) : (
                <div className="new-space-fields">
                  <label className="settings-field">
                    <span>Name</span>
                    <input
                      className="settings-input"
                      value={newSpaceName}
                      onChange={(event) => setNewSpaceName(event.target.value)}
                      placeholder="Exam prep, Algorithms, Week 4..."
                      disabled={isSavingQuestions || hasSavedQuestions}
                    />
                  </label>
                  <label className="settings-field">
                    <span>Description</span>
                    <input
                      className="settings-input"
                      value={newSpaceDescription}
                      onChange={(event) => setNewSpaceDescription(event.target.value)}
                      placeholder="Optional"
                      disabled={isSavingQuestions || hasSavedQuestions}
                    />
                  </label>
                </div>
              )}

              {saveStatus && (
                <p className={`settings-status save-status${saveStatusKind ? ` is-${saveStatusKind}` : ''}`}>
                  {saveStatus}
                </p>
              )}
            </div>

            <button
              type="button"
              className="btn-primary btn-save-questions"
              onClick={saveGeneratedQuestions}
              disabled={isSavingQuestions || hasSavedQuestions || generatedQuestionCount === 0}
            >
              {isSavingQuestions ? (
                'Saving...'
              ) : hasSavedQuestions ? (
                <span className="btn-content">
                  <Check className="btn-icon" aria-hidden="true" />
                  Saved
                </span>
              ) : (
                <span className="btn-content">
                  {saveDestinationMode === 'new' ? (
                    <Plus className="btn-icon" aria-hidden="true" />
                  ) : (
                    <Save className="btn-icon" aria-hidden="true" />
                  )}
                  Save Questions
                </span>
              )}
            </button>
          </div>

          {showChunks && (
            <div className="chunk-panel">
              {generationSummary.chunk_previews.length === 0 ? (
                <p className="chunk-empty">No chunks were generated for this vault.</p>
              ) : (
                <ul className="chunk-list">
                  {generationSummary.chunk_previews.map((chunk) => (
                    <li
                      key={`${chunk.note_path}-${chunk.section_index}-${chunk.chunk_index}`}
                      className={`chunk-item${selectedChunk?.note_path === chunk.note_path && selectedChunk.section_index === chunk.section_index && selectedChunk.chunk_index === chunk.chunk_index ? ' is-selected' : ''}`}
                    >
                      <button
                        type="button"
                        className={`chunk-item-btn${selectedChunk?.note_path === chunk.note_path && selectedChunk.section_index === chunk.section_index && selectedChunk.chunk_index === chunk.chunk_index ? ' is-active' : ''}`}
                        onClick={() => setSelectedChunk(chunk)}
                      >
                        <div className="chunk-item-head">
                          <h3>{chunk.heading}</h3>
                          <span>{chunk.char_count} chars</span>
                        </div>
                        <p className="chunk-meta">
                          {chunk.note_title} • lines {chunk.start_line}-{chunk.end_line}
                        </p>
                        <p className="chunk-preview-text">{chunk.preview_text}</p>
                      </button>

                      {selectedChunk?.note_path === chunk.note_path && selectedChunk.section_index === chunk.section_index && selectedChunk.chunk_index === chunk.chunk_index && (
                        <article className="chunk-result-panel">
                          <h3>
                            Chunk Result • {chunk.heading}
                          </h3>
                          <p className="chunk-result-status">
                            Status: {chunk.llm_result.status}
                          </p>

                          {chunk.llm_result.error ? (
                            <p className="chunk-result-error">{chunk.llm_result.error}</p>
                          ) : (
                            <div className="chunk-result-block">
                              <h4>MCQs</h4>
                              <ul>
                                {chunk.llm_result.questions.map((mcq) => (
                                  <li key={`${mcq.question}-${mcq.correct_answer}`} className="chunk-mcq-item">
                                    <p className="chunk-result-question">{mcq.question}</p>
                                    <ul className="chunk-mcq-options">
                                      <li className={mcq.correct_answer === 'A' ? 'is-correct' : ''}>
                                        <span>A.</span> {mcq.option_a}
                                      </li>
                                      <li className={mcq.correct_answer === 'B' ? 'is-correct' : ''}>
                                        <span>B.</span> {mcq.option_b}
                                      </li>
                                      <li className={mcq.correct_answer === 'C' ? 'is-correct' : ''}>
                                        <span>C.</span> {mcq.option_c}
                                      </li>
                                      <li className={mcq.correct_answer === 'D' ? 'is-correct' : ''}>
                                        <span>D.</span> {mcq.option_d}
                                      </li>
                                    </ul>
                                    <p className="chunk-result-answer">
                                      Correct Answer: {mcq.correct_answer}
                                    </p>
                                  </li>
                                ))}
                              </ul>
                            </div>
                          )}
                        </article>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </section>
      )}

        </main>
      )}
    </div>
  )
}
