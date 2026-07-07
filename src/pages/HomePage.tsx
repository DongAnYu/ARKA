import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { ArrowRight, Check, ChevronDown, FolderOpen, Plus, Save, Sparkles } from 'lucide-react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import arkaLogo from '../assets/arka-logo.svg'

const getWelcomeMessage = (date: Date = new Date()): string => {
  const hour = date.getHours()

  if (hour >= 5 && hour < 12) {
    return 'Good morning — ready to test your recall?'
  }

  if (hour >= 12 && hour < 17) {
    return 'Good afternoon — ready to test your recall?'
  }

  if (hour >= 17 && hour < 22) {
    return 'Good evening — ready to test your recall?'
  }

  return 'Late night — ready to test your recall?'
}

type ViewMode = 'reader' | 'raw'

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

type GenerationProgress = {
  job_id: string
  total_notes: number
  total_chunks: number
  notes_with_chunks: number
  completed_chunks: number
  mcq_generated: number
  progress_percent: number
  is_paused: boolean
  is_cancelled: boolean
  is_finished: boolean
  error: string | null
  summary: GenerationSummary | null
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

export function HomePage() {
  const welcomeMessage = getWelcomeMessage()
  const saveInFlightRef = useRef(false)
  const [vaultPath, setVaultPath] = useState('')
  const [notes, setNotes] = useState<Note[]>([])
  const [selectedNote, setSelectedNote] = useState<Note | null>(null)
  const [viewMode, setViewMode] = useState<ViewMode>('reader')
  const [isLoading, setIsLoading] = useState(false)
  const [isGenerating, setIsGenerating] = useState(false)
  const [isSavingQuestions, setIsSavingQuestions] = useState(false)
  const [generationJobId, setGenerationJobId] = useState<string | null>(null)
  const [generationProgress, setGenerationProgress] = useState<GenerationProgress | null>(null)
  const [generationSummary, setGenerationSummary] = useState<GenerationSummary | null>(null)
  const [showChunks, setShowChunks] = useState(false)
  const [selectedChunk, setSelectedChunk] = useState<ChunkPreview | null>(null)
  const [recallSpaces, setRecallSpaces] = useState<RecallSpace[]>([])
  const [selectedSpaceId, setSelectedSpaceId] = useState(1)
  const [saveDestinationMode, setSaveDestinationMode] = useState<'existing' | 'new'>('existing')
  const [newSpaceName, setNewSpaceName] = useState('')
  const [newSpaceDescription, setNewSpaceDescription] = useState('')
  const [saveStatus, setSaveStatus] = useState('')
  const [saveStatusKind, setSaveStatusKind] = useState<'success' | 'error' | ''>('')
  const [hasSavedQuestions, setHasSavedQuestions] = useState(false)
  const [error, setError] = useState('')

  const generatedQuestionCount =
    generationSummary?.chunk_previews.reduce(
      (total, chunk) => total + chunk.llm_result.questions.length,
      0,
    ) ?? 0

  const selectedSpace = recallSpaces.find((space) => space.id === selectedSpaceId)

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
    loadRecallSpaces()
  }, [])

  const chooseVault = async () => {
    setError('')

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: 'Choose Obsidian Vault',
      })

      if (typeof selected !== 'string') {
        return
      }

      setVaultPath(selected)
      await loadNotes(selected)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to choose vault'
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

    try {
      const data = await invoke<Note[]>('get_notes', { vaultPath: path })
      setNotes(data)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load notes'
      setError(message)
    } finally {
      setIsLoading(false)
    }
  }

  const generatePreview = async () => {
    if (!vaultPath) {
      setError('Choose a vault first before generating preview.')
      return
    }

    setIsGenerating(true)
    setError('')
    setGenerationProgress(null)
    setGenerationSummary(null)
    setShowChunks(false)
    setSelectedChunk(null)

    try {
      const jobId = await invoke<string>('start_preview_generation', {
        vaultPath,
      })
      setGenerationJobId(jobId)
    } catch (err) {
      const message =
        err instanceof Error ? err.message : 'Failed to generate chunk preview'
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

        if (progress.error) {
          setError(progress.error)
        }

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
      const modelConfig = await invoke<any>('load_model_config')
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
    <div className="app-container home-page">
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
          {!vaultPath ? (
            <button
              type="button"
              className="btn-primary"
              onClick={chooseVault}
              disabled={isLoading}
            >
              {isLoading ? (
                'Loading...'
              ) : (
                <span className="btn-content">
                  Choose Vault
                  <ArrowRight className="size-4" aria-hidden="true" />
                </span>
              )}
            </button>
          ) : (
            <>
              <button
                type="button"
                className="btn-primary btn-generate-action"
                onClick={generatePreview}
                disabled={isGenerating}
              >
                {isGenerating ? (
                  'Generating...'
                ) : (
                  <span className="btn-content">
                    <Sparkles className="size-4" aria-hidden="true" />
                    Generate Preview
                  </span>
                )}
              </button>
              <button
                type="button"
                className="btn-secondary btn-vault-action"
                onClick={chooseVault}
                disabled={isLoading || isGenerating}
              >
                <span className="btn-content">
                  <FolderOpen className="size-4" aria-hidden="true" />
                  Change Vault
                </span>
              </button>
            </>
          )}
        </div>
      </div>

      {vaultPath && <p className="vault-path">{vaultPath}</p>}
      {error && <div className="error-banner">{error}</div>}

      {generationProgress && !generationProgress.is_finished && (
        <section className="generation-progress" aria-live="polite">
          <div className="generation-progress-head">
            <h2>Generating Preview</h2>
            <span>{generationProgress.progress_percent}%</span>
          </div>

          <div className="generation-bar-track">
            <div
              className="generation-bar-fill"
              style={{ width: `${generationProgress.progress_percent}%` }}
            />
          </div>

          <div className="summary-grid">
            <p>
              <span>Chunks Completed</span>
              <strong>{generationProgress.completed_chunks}/{generationProgress.total_chunks}</strong>
            </p>
            <p>
              <span>MCQs Generated</span>
              <strong>{generationProgress.mcq_generated}</strong>
            </p>
            <p>
              <span>Status</span>
              <strong>{generationProgress.is_paused ? 'Paused' : 'Running'}</strong>
            </p>
          </div>

          <div className="generation-actions">
            <button
              type="button"
              className="btn-pause"
              onClick={togglePauseGeneration}
            >
              {generationProgress.is_paused ? '▶ Resume' : '⏸ Pause'}
            </button>
            <button
              type="button"
              className="btn-cancel"
              onClick={cancelGeneration}
            >
              ✕ Cancel
            </button>
          </div>
        </section>
      )}

      {generationSummary && (
        <section className="generation-summary" aria-live="polite">
          <div className="generation-summary-head">
            <h2>Chunk Preview Summary</h2>
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
              {showChunks ? 'Hide Chunks' : 'View Chunks'}
            </button>
          </div>
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
                  <p>{generatedQuestionCount} generated questions ready</p>
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
                  <span>Recall space</span>
                  <select
                    className="recall-space-select"
                    value={selectedSpaceId}
                    onChange={(event) => setSelectedSpaceId(Number(event.target.value))}
                    disabled={isSavingQuestions || hasSavedQuestions || recallSpaces.length === 0}
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

              {saveStatus && (
                <p className={`settings-status save-status${saveStatusKind ? ` is-${saveStatusKind}` : ''}`}>
                  {saveStatus}
                </p>
              )}
            </div>
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

      {selectedNote ? (
        <article className="note-detail">
          <button
            type="button"
            className="btn-back"
            onClick={() => setSelectedNote(null)}
          >
            ← Back to notes
          </button>
          <div className="note-detail-header">
            <h2 className="note-detail-title">{selectedNote.title}</h2>
            <div className="view-toggle" role="group" aria-label="View mode">
              <button
                type="button"
                className={`view-toggle-btn${viewMode === 'reader' ? ' is-active' : ''}`}
                onClick={() => setViewMode('reader')}
                aria-pressed={viewMode === 'reader'}
                aria-label="Reading view"
                title="Reading view"
              >
                <svg
                  className="view-toggle-icon"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="M12 7v14" />
                  <path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z" />
                </svg>
              </button>
              <button
                type="button"
                className={`view-toggle-btn${viewMode === 'raw' ? ' is-active' : ''}`}
                onClick={() => setViewMode('raw')}
                aria-pressed={viewMode === 'raw'}
                aria-label="Source view"
                title="Source view"
              >
                <svg
                  className="view-toggle-icon"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="m16 18 6-6-6-6" />
                  <path d="m8 6-6 6 6 6" />
                </svg>
              </button>
            </div>
          </div>
          {viewMode === 'reader' ? (
            <div className="note-reader">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {selectedNote.content}
              </ReactMarkdown>
            </div>
          ) : (
            <pre className="note-detail-content">{selectedNote.content}</pre>
          )}
        </article>
      ) : (
        <div className="notes-container" aria-live="polite">
          {notes.length === 0 ? (
            <p className="empty-state">Choose a vault to load your notes.</p>
          ) : (
            <>
              <p className="notes-count">Found {notes.length} Notes</p>
              <ul className="note-list">
                {notes.map((note) => (
                  <li key={note.path}>
                    <button
                      type="button"
                      className="note-item"
                      onClick={() => {
                        setViewMode('reader')
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
      )}
    </div>
  )
}
