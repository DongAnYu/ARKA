import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { Link } from 'react-router-dom'
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
import { BackToHome } from '../components/BackToHome'
import { useGeneration } from '../generation/context'
import type { ChunkPreview, Note } from '../generation/types'
import {
  MODEL_CONFIG_UPDATED_EVENT,
  type PersistedModelConfig,
} from '../modelConfig'

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

const providerNames: Record<string, string> = {
  ollama: 'Ollama',
  openai: 'OpenAI',
  openrouter: 'OpenRouter',
}

const describeModel = (provider: string, model: string) => {
  const selectedModel = model.trim()
  if (!selectedModel) {
    return 'No model selected'
  }

  const providerName = providerNames[provider] ?? provider
  return providerName ? `${providerName} · ${selectedModel}` : selectedModel
}

export function HomePage() {
  const welcomeMessage = getWelcomeMessage()
  const saveInFlightRef = useRef(false)
  const {
    notes,
    selectedNote,
    generationMode,
    generationProgress,
    generationSummary,
    generationError,
    isGenerating,
    setSourceNotes,
    selectNote,
    clearSelectedNote,
    setGenerationMode,
    startGeneration,
    togglePauseGeneration,
    cancelGeneration,
  } = useGeneration()
  const [isLoading, setIsLoading] = useState(false)
  const [isSavingQuestions, setIsSavingQuestions] = useState(false)
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
  const [modelConfig, setModelConfig] = useState<PersistedModelConfig | null>(null)
  const [isLoadingModelConfig, setIsLoadingModelConfig] = useState(true)
  const [modelConfigError, setModelConfigError] = useState('')

  const generatedQuestionCount =
    generationSummary?.chunk_previews.reduce(
      (total, chunk) => total + chunk.llm_result.questions.length,
      0,
    ) ?? 0

  const selectedSpace = recallSpaces.find((space) => space.id === selectedSpaceId)
  const visibleError = error || generationError
  const isLlmReady = Boolean(modelConfig?.selected_model.trim())
  const isEmbeddingReady = Boolean(modelConfig?.embedding_selected_model.trim())
  const areRequiredModelsReady =
    generationMode === 'default'
      ? isLlmReady
      : generationMode === 'graph'
        ? isLlmReady && isEmbeddingReady
        : false
  const canStartGeneration =
    areRequiredModelsReady && !isLoadingModelConfig && !modelConfigError

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

  useEffect(() => {
    let cancelled = false

    void invoke<PersistedModelConfig>('load_model_config')
      .then((config) => {
        if (cancelled) {
          return
        }

        setModelConfig(config)
        setModelConfigError('')
      })
      .catch(() => {
        if (!cancelled) {
          setModelConfigError('Could not read the saved model configuration.')
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoadingModelConfig(false)
        }
      })

    const handleConfigUpdate = (event: Event) => {
      const configEvent = event as CustomEvent<PersistedModelConfig>
      setModelConfig(configEvent.detail)
      setModelConfigError('')
      setIsLoadingModelConfig(false)
    }

    window.addEventListener(MODEL_CONFIG_UPDATED_EVENT, handleConfigUpdate)

    return () => {
      cancelled = true
      window.removeEventListener(MODEL_CONFIG_UPDATED_EVENT, handleConfigUpdate)
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

      await loadNotes(selected)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to choose markdown file'
      setError(message)
    }
  }

  const loadNotes = async (path: string) => {
    setIsLoading(true)
    setError('')
    setSourceNotes(path, [])
    setShowChunks(false)
    setSelectedChunk(null)
    setSaveStatus('')
    setSaveStatusKind('')
    setHasSavedQuestions(false)

    try {
      const data = await invoke<Note[]>('get_notes', { vaultPath: path })
      setSourceNotes(path, data)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load notes'
      setError(message)
    } finally {
      setIsLoading(false)
    }
  }

  const returnToHome = () => {
    clearSelectedNote()
    setError('')
    setShowChunks(false)
    setSelectedChunk(null)
    setSaveStatus('')
    setSaveStatusKind('')
    setHasSavedQuestions(false)
  }

  const generatePreview = async () => {
    setError('')
    setShowChunks(false)
    setSelectedChunk(null)
    await startGeneration()
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
      const latestModelConfig = await invoke<PersistedModelConfig>('load_model_config')
      const modelName = latestModelConfig.selected_model || 'unknown'

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

  const generationPercent = generationProgress?.progress_percent ?? 0
  const completedWork = generationProgress?.completed_chunks ?? 0
  const totalWork = generationProgress?.total_chunks ?? 0
  const skippedWork = generationProgress?.failed_chunks ?? 0
  const successfulWork = Math.max(completedWork - skippedWork, 0)
  const questionsGenerated = generationProgress?.mcq_generated ?? 0
  const recallQuestions = generationProgress?.recall_mcq_generated ?? 0
  const relationalQuestions = generationProgress?.relational_mcq_generated ?? 0
  const workScale = Math.max(totalWork, 1)
  const questionScale = Math.max(questionsGenerated, 1)
  const graphPhases = [
    'Extracting knowledge',
    'Building knowledge graph',
    'Resolving entities',
    'Generating questions',
  ]
  const currentGraphPhase = generationProgress?.phase_label ?? graphPhases[0]
  const currentGraphPhaseIndex = Math.max(graphPhases.indexOf(currentGraphPhase), 0)
  const finalCompletionPercent = generationProgress?.error
    ? generationProgress.progress_percent
    : 100
  const finalSkippedWork = generationProgress?.failed_chunks ?? 0
  const skippedWarningMessages = Array.from(
    new Set((generationProgress?.warnings ?? []).map((warning) => warning.message)),
  )
  const visibleSkippedWarnings = skippedWarningMessages.slice(0, 3)
  const additionalSkippedWarnings = skippedWarningMessages.length - visibleSkippedWarnings.length
  const finalCompletedWork = Math.max(
    (generationSummary?.total_chunks ?? 0) - finalSkippedWork,
    0,
  )

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

          {visibleError && <p className="error-text" role="alert">{visibleError}</p>}

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
                        onClick={() => selectNote(note)}
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
            <BackToHome onActivate={returnToHome} disabled={isGenerating} />

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

          {visibleError && <p className="error-text" role="alert">{visibleError}</p>}

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
                <div
                  className="generation-commit"
                  aria-live="polite"
                  aria-busy={isLoadingModelConfig}
                >
                  <div className="generation-commit-copy">
                    <div className="generation-commit-heading">
                      <strong>
                        {generationMode === 'graph' ? 'Deep thinking' : 'Default generation'}
                      </strong>
                      <span>
                        {generationMode === 'graph'
                          ? 'Both models are required for connected, concept-heavy questions.'
                          : 'One language model is required for a quick, focused study set.'}
                      </span>
                    </div>

                    <dl className="generation-model-readiness">
                      <div
                        className={`generation-model-row${
                          !isLoadingModelConfig && !modelConfigError && isLlmReady
                            ? ' is-ready'
                            : ' needs-setup'
                        }`}
                      >
                        <dt>
                          <Sparkles aria-hidden="true" />
                          <span>Language model</span>
                        </dt>
                        <dd>
                          <strong
                            title={
                              modelConfig
                                ? describeModel(modelConfig.provider, modelConfig.selected_model)
                                : undefined
                            }
                          >
                            {isLoadingModelConfig
                              ? 'Checking configuration…'
                              : modelConfigError
                                ? 'Unable to verify'
                                : modelConfig
                                  ? describeModel(modelConfig.provider, modelConfig.selected_model)
                                  : 'No model selected'}
                          </strong>
                          <span className="generation-model-state">
                            {!isLoadingModelConfig && !modelConfigError && isLlmReady ? (
                              <Check aria-hidden="true" />
                            ) : (
                              <AlertTriangle aria-hidden="true" />
                            )}
                            {isLoadingModelConfig
                              ? 'Checking'
                              : modelConfigError
                                ? 'Unavailable'
                                : isLlmReady
                                  ? 'Ready'
                                  : 'Not configured'}
                          </span>
                        </dd>
                      </div>

                      {generationMode === 'graph' && (
                        <div
                          className={`generation-model-row${
                            !isLoadingModelConfig && !modelConfigError && isEmbeddingReady
                              ? ' is-ready'
                              : ' needs-setup'
                          }`}
                        >
                          <dt>
                            <GitBranch aria-hidden="true" />
                            <span>Embedding model</span>
                          </dt>
                          <dd>
                            <strong
                              title={
                                modelConfig
                                  ? describeModel(
                                      modelConfig.embedding_provider,
                                      modelConfig.embedding_selected_model,
                                    )
                                  : undefined
                              }
                            >
                              {isLoadingModelConfig
                                ? 'Checking configuration…'
                                : modelConfigError
                                  ? 'Unable to verify'
                                  : modelConfig
                                    ? describeModel(
                                        modelConfig.embedding_provider,
                                        modelConfig.embedding_selected_model,
                                      )
                                    : 'No model selected'}
                            </strong>
                            <span className="generation-model-state">
                              {!isLoadingModelConfig &&
                              !modelConfigError &&
                              isEmbeddingReady ? (
                                <Check aria-hidden="true" />
                              ) : (
                                <AlertTriangle aria-hidden="true" />
                              )}
                              {isLoadingModelConfig
                                ? 'Checking'
                                : modelConfigError
                                  ? 'Unavailable'
                                  : isEmbeddingReady
                                    ? 'Ready'
                                    : 'Not configured'}
                            </span>
                          </dd>
                        </div>
                      )}
                    </dl>

                    {!isLoadingModelConfig && !modelConfigError && !areRequiredModelsReady && (
                      <Link className="generation-model-settings-link" to="/models">
                        Configure model settings
                        <ArrowRight aria-hidden="true" />
                      </Link>
                    )}
                    {modelConfigError && (
                      <span className="generation-model-config-error">{modelConfigError}</span>
                    )}
                  </div>

                  <div className="generation-commit-actions">
                    <button
                      type="button"
                      className="btn-primary btn-start-generation"
                      onClick={generatePreview}
                      disabled={!canStartGeneration}
                      aria-describedby={!canStartGeneration ? 'generation-readiness-help' : undefined}
                    >
                      <span className="btn-content">
                        <Sparkles className="size-4" aria-hidden="true" />
                        Generate questions
                      </span>
                    </button>
                    {!canStartGeneration && (
                      <span className="generation-action-help" id="generation-readiness-help">
                        {isLoadingModelConfig
                          ? 'Checking your saved model settings…'
                          : 'Configure the required models to continue.'}
                      </span>
                    )}
                  </div>
                </div>
              )}
            </section>
          )}

          {isGenerating && (
            <section className="generation-progress generation-progress-focused" aria-live="polite">
              <header className="generation-live-banner">
                <span
                  className={`generation-live-pulse${generationProgress?.is_paused ? ' is-paused' : ''}`}
                  aria-hidden="true"
                >
                  {generationMode === 'graph' ? <GitBranch /> : <Sparkles />}
                </span>
                <div className="generation-live-copy">
                  <span className="generation-live-kicker">
                    {generationProgress?.is_paused ? 'Background activity paused' : 'AI working in background'}
                  </span>
                  <h2 key={generationProgress?.activity ?? 'preparing'}>
                    {generationProgress?.is_paused
                      ? generationProgress.activity
                        ? `Paused · ${generationProgress.activity}`
                        : 'Generation is paused'
                      : generationProgress?.activity ?? 'Preparing your note'}
                  </h2>
                </div>
                <span className="generation-phase-badge">
                  {generationProgress?.phase_label ?? (generationMode === 'graph' ? 'Starting graph' : 'Generating')}
                </span>
              </header>

              <div className="generation-dashboard">
                <div className="generation-phase-visual">
                  <div className="generation-panel-heading">
                    <span>Overall progress</span>
                    <strong>{generationMode === 'graph' ? 'Graph pipeline' : 'Question pipeline'}</strong>
                  </div>

                  <div
                    className="generation-progress-ring"
                    role="progressbar"
                    aria-label="Question generation progress"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={generationPercent}
                  >
                    <svg viewBox="0 0 176 176" aria-hidden="true">
                      <circle className="generation-ring-track" cx="88" cy="88" r="72" />
                      <circle
                        className="generation-ring-value"
                        cx="88"
                        cy="88"
                        r="72"
                        pathLength="1"
                        style={{ strokeDashoffset: 1 - generationPercent / 100 }}
                      />
                    </svg>
                    <div className="generation-ring-label">
                      <strong key={generationPercent}>{generationPercent}%</strong>
                      <span>complete</span>
                    </div>
                  </div>

                  {generationMode === 'graph' ? (
                    <ol className="generation-phase-list" aria-label="Graph generation phases">
                      {graphPhases.map((phase, index) => (
                        <li
                          key={phase}
                          className={index < currentGraphPhaseIndex ? 'is-complete' : index === currentGraphPhaseIndex ? 'is-current' : ''}
                        >
                          <span aria-hidden="true">{index < currentGraphPhaseIndex ? <Check /> : index + 1}</span>
                          <div>
                            <strong>{phase}</strong>
                            <small>{index < currentGraphPhaseIndex ? 'Complete' : index === currentGraphPhaseIndex ? 'In progress' : 'Waiting'}</small>
                          </div>
                        </li>
                      ))}
                    </ol>
                  ) : (
                    <p className="generation-phase-note">
                      Processing chunk {generationProgress?.current_chunk ?? 0} of {totalWork || '—'}
                    </p>
                  )}
                </div>

                <div className="generation-output-visual">
                  <div className="generation-panel-heading generation-output-heading">
                    <span>Question output</span>
                    <span className="generation-live-label"><i aria-hidden="true" /> Live</span>
                  </div>

                  <div className="generation-question-total">
                    <strong key={questionsGenerated}>{questionsGenerated}</strong>
                    <span>questions built</span>
                  </div>

                  <div className="generation-chart-group">
                    {generationMode === 'graph' ? (
                      <>
                        <div className="generation-chart-row">
                          <div><span>Relational</span><strong>{relationalQuestions}</strong></div>
                          <div className="generation-chart-track"><span className="is-relational" style={{ transform: `scaleX(${relationalQuestions / questionScale})` }} /></div>
                        </div>
                        <div className="generation-chart-row">
                          <div><span>Recall</span><strong>{recallQuestions}</strong></div>
                          <div className="generation-chart-track"><span className="is-recall" style={{ transform: `scaleX(${recallQuestions / questionScale})` }} /></div>
                        </div>
                      </>
                    ) : (
                      <div className="generation-chart-row">
                        <div><span>Generated</span><strong>{questionsGenerated}</strong></div>
                        <div className="generation-chart-track"><span className="is-relational" style={{ transform: `scaleX(${questionsGenerated / workScale})` }} /></div>
                      </div>
                    )}
                  </div>

                  <div className="generation-work-summary">
                    <div className="generation-work-heading">
                      <span>{generationMode === 'graph' ? 'Knowledge work' : 'Chunk work'}</span>
                      <strong>{completedWork} / {totalWork || '—'}</strong>
                    </div>
                    <div className="generation-work-bar" aria-label={`${successfulWork} completed, ${skippedWork} skipped`}>
                      <span className="is-complete" style={{ transform: `scaleX(${successfulWork / workScale})` }} />
                      <span
                        className="is-skipped"
                        style={{ left: `${(successfulWork / workScale) * 100}%`, transform: `scaleX(${skippedWork / workScale})` }}
                      />
                    </div>
                    <div className="generation-work-legend">
                      <span><i className="is-complete" aria-hidden="true" />{successfulWork} completed</span>
                      <span><i className="is-skipped" aria-hidden="true" />{skippedWork} skipped</span>
                    </div>
                  </div>
                </div>
              </div>

              {(generationProgress?.failed_chunks ?? 0) > 0 && (
                <p className="generation-inline-warning" role="status">
                  <AlertTriangle aria-hidden="true" />
                  {generationProgress?.failed_chunks === 1
                    ? '1 chunk could not be generated and was skipped. Continuing with the rest.'
                    : `${generationProgress?.failed_chunks} chunks could not be generated and were skipped. Continuing with the rest.`}
                </p>
              )}

              <footer className="generation-dashboard-footer">
                <span>
                  <Zap aria-hidden="true" />
                  You can leave this screen open while ARKA keeps working.
                </span>
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
              </footer>
            </section>
          )}

      {generationSummary && (
        <section className="generation-summary generation-complete-dashboard" aria-live="polite">
          <header className="generation-complete-banner">
            <span
              className={`generation-complete-symbol${generationProgress?.error ? ' is-error' : ''}`}
              aria-hidden="true"
            >
              {generationProgress?.error ? <AlertTriangle /> : <Check />}
            </span>
            <div>
              <h2>
                {generationProgress?.error
                  ? 'Generation stopped early'
                  : 'Your questions are ready'}
              </h2>
              <p>
                {generatedQuestionCount}{' '}
                {generatedQuestionCount === 1 ? 'question is' : 'questions are'} ready to review and save.
              </p>
            </div>
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
          </header>

          <div className="generation-complete-overview">
            <div className="generation-phase-visual generation-complete-visual">
              <div className="generation-panel-heading">
                <span>Completion</span>
                <strong>{generationProgress?.error ? 'Stopped early' : 'Generation finished'}</strong>
              </div>

              <div
                className={`generation-progress-ring${generationProgress?.error ? ' is-error' : ' is-complete'}`}
                role="progressbar"
                aria-label="Completed generation progress"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={finalCompletionPercent}
              >
                <svg viewBox="0 0 176 176" aria-hidden="true">
                  <circle className="generation-ring-track" cx="88" cy="88" r="72" />
                  <circle
                    className="generation-ring-value"
                    cx="88"
                    cy="88"
                    r="72"
                    pathLength="1"
                    style={{ strokeDashoffset: 1 - finalCompletionPercent / 100 }}
                  />
                </svg>
                <div className="generation-ring-label">
                  <strong>{finalCompletionPercent}%</strong>
                  <span>{generationProgress?.error ? 'reached' : 'complete'}</span>
                </div>
              </div>

              <p className="generation-complete-message">
                {generationProgress?.error
                  ? 'Your completed questions are preserved and can still be saved.'
                  : 'Generation finished. Choose a Recall Space to continue.'}
              </p>
            </div>

            <div className="generation-complete-insights">
              <div className="generation-panel-heading">
                <span>Generation insights</span>
                <strong>{generationMode === 'graph' ? 'Graph pipeline' : 'Question pipeline'}</strong>
              </div>

              <dl className="generation-insight-list">
                <div>
                  <dt>Questions created</dt>
                  <dd>{generatedQuestionCount}</dd>
                </div>
                <div>
                  <dt>Notes covered</dt>
                  <dd>{generationSummary.notes_with_chunks} / {generationSummary.total_notes}</dd>
                </div>
                <div>
                  <dt>{generationMode === 'graph' ? 'Knowledge work completed' : 'Chunks completed'}</dt>
                  <dd>{finalCompletedWork} / {generationSummary.total_chunks}</dd>
                </div>
                <div className={finalSkippedWork > 0 ? 'has-warning' : ''}>
                  <dt>Skipped work</dt>
                  <dd>{finalSkippedWork}</dd>
                </div>
              </dl>

              {generationMode === 'graph' && generatedQuestionCount > 0 && (
                <div className="generation-complete-composition">
                  <div className="generation-work-heading">
                    <span>Question composition</span>
                    <strong>{generatedQuestionCount} total</strong>
                  </div>
                  <div className="generation-composition-bar" aria-label={`${relationalQuestions} relational and ${recallQuestions} recall questions`}>
                    <span
                      className="is-relational"
                      style={{ transform: `scaleX(${relationalQuestions / generatedQuestionCount})` }}
                    />
                    <span
                      className="is-recall"
                      style={{
                        left: `${(relationalQuestions / generatedQuestionCount) * 100}%`,
                        transform: `scaleX(${recallQuestions / generatedQuestionCount})`,
                      }}
                    />
                  </div>
                  <div className="generation-work-legend">
                    <span><i className="is-relational" aria-hidden="true" />{relationalQuestions} relational</span>
                    <span><i className="is-recall" aria-hidden="true" />{recallQuestions} recall</span>
                  </div>
                </div>
              )}

              {generationProgress?.error ? (
                <div className="generation-result-notice is-error" role="alert">
                  <AlertTriangle aria-hidden="true" />
                  <div>
                    <strong>Provider request failed</strong>
                    <p>{generationProgress.error.message}</p>
                  </div>
                </div>
              ) : finalSkippedWork > 0 ? (
                <div className="generation-result-notice is-warning" role="status">
                  <AlertTriangle aria-hidden="true" />
                  <div>
                    <strong>
                      Completed with {finalSkippedWork}{' '}
                      {finalSkippedWork === 1 ? 'skipped chunk' : 'skipped chunks'}
                    </strong>
                    <p>Questions from the remaining work are ready to save.</p>
                    {visibleSkippedWarnings.length > 0 && (
                      <div className="generation-result-details">
                        <ul>
                          {visibleSkippedWarnings.map((message) => (
                            <li key={message}>{message}</li>
                          ))}
                        </ul>
                        {additionalSkippedWarnings > 0 && (
                          <small>
                            {additionalSkippedWarnings} more distinct{' '}
                            {additionalSkippedWarnings === 1 ? 'issue' : 'issues'}
                          </small>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              ) : null}
            </div>
          </div>

          <div className="generation-summary-actions generation-complete-save">
            <div className="generation-save-heading">
              <span className="generation-save-symbol" aria-hidden="true">
                <Save />
              </span>
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

            <div className={`generation-save-controls${saveDestinationMode === 'new' ? ' is-new-space' : ''}`}>
              {saveDestinationMode === 'existing' ? (
                <label className="recall-space-select-wrap">
                  <span>{isLoadingRecallSpaces ? 'Loading spaces…' : 'Destination'}</span>
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
                    {saveDestinationMode === 'new' ? 'Create Space & Save' : 'Save Questions'}
                  </span>
                )}
              </button>
            </div>

            {saveStatus && (
              <p className={`settings-status save-status${saveStatusKind ? ` is-${saveStatusKind}` : ''}`}>
                {saveStatus}
              </p>
            )}
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
