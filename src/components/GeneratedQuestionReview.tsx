import { useEffect, useRef, useState } from 'react'
import {
  ArrowRight,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Sparkles,
  X,
} from 'lucide-react'
import {
  answerOptions,
  getReviewQuestionError,
  type ReviewDecision,
  type ReviewQuestionDraft,
} from '../generation/review'
import { commands } from '../commands/commands'
import { getAriaKeyShortcut, getShortcut } from '../shortcuts/defaultShortcuts'
import { useKeyboardShortcuts } from '../shortcuts/useKeyboardShortcuts'

type EditableQuestionField =
  | 'question'
  | 'option_a'
  | 'option_b'
  | 'option_c'
  | 'option_d'
  | 'correct_answer'

type RecallSpaceOption = {
  id: number
  name: string
}

type GeneratedQuestionReviewProps = {
  questions: ReviewQuestionDraft[]
  destinationName: string
  saveDestinationMode: 'existing' | 'new'
  onSaveDestinationModeChange: (mode: 'existing' | 'new') => void
  recallSpaces: RecallSpaceOption[]
  isLoadingRecallSpaces: boolean
  selectedSpaceId: number
  onSelectedSpaceChange: (spaceId: number) => void
  newSpaceName: string
  onNewSpaceNameChange: (name: string) => void
  newSpaceDescription: string
  onNewSpaceDescriptionChange: (description: string) => void
  isSaving: boolean
  isGenerationActive: boolean
  initialActiveIndex: number
  onUpdateQuestion: (
    reviewId: string,
    updates: Partial<ReviewQuestionDraft>,
  ) => void
  onKeepRemaining: () => void
  onSaveKept: () => void
  onFinishWithoutSaving: () => void
  onClose: (activeIndex: number) => void
}

export function GeneratedQuestionReview({
  questions,
  destinationName,
  saveDestinationMode,
  onSaveDestinationModeChange,
  recallSpaces,
  isLoadingRecallSpaces,
  selectedSpaceId,
  onSelectedSpaceChange,
  newSpaceName,
  onNewSpaceNameChange,
  newSpaceDescription,
  onNewSpaceDescriptionChange,
  isSaving,
  isGenerationActive,
  initialActiveIndex,
  onUpdateQuestion,
  onKeepRemaining,
  onSaveKept,
  onFinishWithoutSaving,
  onClose,
}: GeneratedQuestionReviewProps) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const maxInitialIndex = isGenerationActive
    ? questions.length
    : Math.max(questions.length - 1, 0)
  const [activeIndex, setActiveIndex] = useState(
    Math.min(Math.max(initialActiveIndex, 0), maxInitialIndex),
  )
  const [validationMessage, setValidationMessage] = useState('')

  const lastAvailableIndex = Math.max(questions.length - 1, 0)
  const displayedActiveIndex = isGenerationActive
    ? activeIndex
    : Math.min(activeIndex, lastAvailableIndex)
  const isCaughtUp = isGenerationActive && activeIndex >= questions.length
  const activeQuestion = questions[
    Math.min(displayedActiveIndex, lastAvailableIndex)
  ]
  const keptCount = questions.filter((question) => question.decision === 'kept').length
  const discardedCount = questions.filter(
    (question) => question.decision === 'discarded',
  ).length
  const remainingCount = questions.length - keptCount - discardedCount
  const decidedCount = questions.length - remainingCount
  const reviewPercent = questions.length
    ? Math.round((decidedCount / questions.length) * 100)
    : 0

  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) {
      return
    }

    dialog.showModal()
    return () => {
      if (dialog.open) {
        dialog.close()
      }
    }
  }, [])

  const updateField = (field: EditableQuestionField, value: string) => {
    if (!activeQuestion || isCaughtUp) {
      return
    }

    setValidationMessage('')
    onUpdateQuestion(activeQuestion.reviewId, { [field]: value })
  }

  const decideQuestion = (decision: Exclude<ReviewDecision, 'pending'>) => {
    if (!activeQuestion || isCaughtUp) {
      return false
    }

    const nextDecision: ReviewDecision =
      activeQuestion.decision === decision ? 'pending' : decision

    if (nextDecision === 'kept') {
      const message = getReviewQuestionError(activeQuestion)
      if (message) {
        setValidationMessage(message)
        return false
      }
    }

    onUpdateQuestion(activeQuestion.reviewId, { decision: nextDecision })
    setValidationMessage('')
    return true
  }

  const keepRemaining = () => {
    const invalidIndex = questions.findIndex(
      (question) =>
        question.decision === 'pending' && Boolean(getReviewQuestionError(question)),
    )

    if (invalidIndex !== -1) {
      setActiveIndex(invalidIndex)
      setValidationMessage(getReviewQuestionError(questions[invalidIndex]))
      return
    }

    onKeepRemaining()
    setValidationMessage('')
  }

  const destinationLabel = destinationName || 'No Recall Space selected'
  const hasValidDestination =
    saveDestinationMode === 'new'
      ? Boolean(newSpaceName.trim())
      : Boolean(recallSpaces.some((space) => space.id === selectedSpaceId))

  const closeReview = () => {
    if (!isSaving) {
      onClose(displayedActiveIndex)
    }
  }

  const goToPreviousQuestion = () => {
    if (displayedActiveIndex === 0 || isSaving) {
      return false
    }

    setActiveIndex(Math.max(0, displayedActiveIndex - 1))
    setValidationMessage('')
    return true
  }

  const goToNextQuestion = () => {
    if (isSaving || isCaughtUp) {
      return false
    }

    if (displayedActiveIndex === lastAvailableIndex) {
      if (!isGenerationActive) {
        return false
      }

      setActiveIndex(questions.length)
      setValidationMessage('')
      return true
    }

    setActiveIndex(Math.min(lastAvailableIndex, displayedActiveIndex + 1))
    setValidationMessage('')
    return true
  }

  useKeyboardShortcuts({
    scope: 'question-review',
    enabled: Boolean(activeQuestion) && !isSaving,
    handlers: {
      [commands.reviewPrevious]: goToPreviousQuestion,
      [commands.reviewNext]: goToNextQuestion,
      [commands.reviewDiscard]: () => decideQuestion('discarded'),
      [commands.reviewKeep]: () => decideQuestion('kept'),
    },
  })

  if (!activeQuestion) {
    return null
  }

  const previousShortcut = getShortcut(commands.reviewPrevious)
  const nextShortcut = getShortcut(commands.reviewNext)
  const discardShortcut = getShortcut(commands.reviewDiscard)
  const keepShortcut = getShortcut(commands.reviewKeep)

  return (
    <dialog
      ref={dialogRef}
      className="question-review-dialog"
      aria-labelledby="question-review-title"
      onCancel={(event) => {
        event.preventDefault()
        closeReview()
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          closeReview()
        }
      }}
    >
      <section className="question-review">
        <header className="question-review-header">
          <div className="question-review-title-group">
            <div>
              <h3 id="question-review-title">Question review</h3>
              <p>
                {isGenerationActive
                  ? 'Review complete drafts while ARKA keeps generating in the background.'
                  : 'Refine each draft before anything is added to your library.'}
              </p>
            </div>
            <div className="question-review-counts" aria-live="polite">
              <span className="is-kept"><strong>{keptCount}</strong> kept</span>
              <span className="is-discarded"><strong>{discardedCount}</strong> discarded</span>
              <span className="is-remaining"><strong>{remainingCount}</strong> remaining</span>
              {isGenerationActive && (
                <span className="is-generating"><i aria-hidden="true" />Generating more…</span>
              )}
            </div>
          </div>

          <div
            className={`question-review-destination-panel${hasValidDestination ? '' : ' is-missing'}`}
          >
            <div className="question-review-destination-heading">
              <FolderOpen aria-hidden="true" />
              <div>
                <span>Destination</span>
                <strong title={destinationLabel}>{destinationLabel}</strong>
              </div>
            </div>

            <div className="question-review-destination-controls">
              <div className="save-mode-toggle" role="group" aria-label="Save destination type">
                <button
                  type="button"
                  className={`save-mode-btn${saveDestinationMode === 'existing' ? ' is-active' : ''}`}
                  onClick={() => onSaveDestinationModeChange('existing')}
                  disabled={isSaving}
                >
                  Existing
                </button>
                <button
                  type="button"
                  className={`save-mode-btn${saveDestinationMode === 'new' ? ' is-active' : ''}`}
                  onClick={() => onSaveDestinationModeChange('new')}
                  disabled={isSaving}
                >
                  New
                </button>
              </div>

              {saveDestinationMode === 'existing' ? (
                <div className="question-review-space-select">
                  <select
                    className="recall-space-select"
                    value={selectedSpaceId}
                    onChange={(event) => onSelectedSpaceChange(Number(event.target.value))}
                    disabled={isLoadingRecallSpaces || isSaving || recallSpaces.length === 0}
                    aria-label="Recall Space destination"
                  >
                    {isLoadingRecallSpaces ? (
                      <option>Loading spaces…</option>
                    ) : recallSpaces.length > 0 ? (
                      recallSpaces.map((space) => (
                        <option key={space.id} value={space.id}>
                          {space.name}
                        </option>
                      ))
                    ) : (
                      <option>No Recall Spaces available</option>
                    )}
                  </select>
                  <ChevronDown className="recall-space-chevron" aria-hidden="true" />
                </div>
              ) : (
                <div className="question-review-new-space-fields">
                  <label>
                    <span>Name</span>
                    <input
                      className="settings-input"
                      value={newSpaceName}
                      onChange={(event) => onNewSpaceNameChange(event.target.value)}
                      placeholder="Exam prep, Algorithms, Week 4..."
                      disabled={isSaving}
                    />
                  </label>
                  <label>
                    <span>Description</span>
                    <input
                      className="settings-input"
                      value={newSpaceDescription}
                      onChange={(event) => onNewSpaceDescriptionChange(event.target.value)}
                      placeholder="Optional"
                      disabled={isSaving}
                    />
                  </label>
                </div>
              )}
            </div>

            {!hasValidDestination && !isLoadingRecallSpaces && (
              <p className="question-review-destination-hint">
                {saveDestinationMode === 'new'
                  ? 'Name the new Recall Space before adding questions.'
                  : 'Choose an existing Recall Space or create a new one.'}
              </p>
            )}
          </div>

          <div className="question-review-progress" aria-hidden="true">
            <span style={{ transform: `scaleX(${reviewPercent / 100})` }} />
          </div>

          <div className="question-review-header-actions">
            {remainingCount > 0 ? (
              <button
                type="button"
                className="btn-secondary question-review-keep-remaining"
                onClick={keepRemaining}
                disabled={isSaving}
              >
                <CheckCheck aria-hidden="true" />
                Keep remaining
              </button>
            ) : isGenerationActive ? (
              <span className="question-review-waiting">
                <i aria-hidden="true" />
                Waiting for more
              </span>
            ) : keptCount > 0 ? (
              <button
                type="button"
                className="btn-primary question-review-finalize"
                onClick={onSaveKept}
                disabled={isSaving || !hasValidDestination}
              >
                {isSaving ? (
                  'Adding questions…'
                ) : (
                  <>
                    Add {keptCount} {keptCount === 1 ? 'Question' : 'Questions'}
                    <ArrowRight aria-hidden="true" />
                  </>
                )}
              </button>
            ) : (
              <button
                type="button"
                className="btn-primary question-review-finalize"
                onClick={onFinishWithoutSaving}
                disabled={isSaving}
              >
                Finish review
                <ArrowRight aria-hidden="true" />
              </button>
            )}
            <button
              type="button"
              className="question-review-close"
              onClick={closeReview}
              disabled={isSaving}
              aria-label="Close question review"
            >
              <X aria-hidden="true" />
            </button>
          </div>
        </header>

        <div className="question-review-workspace">
          <div className="question-review-position-row">
            <div>
              {isCaughtUp ? (
                <span className="question-review-position">All {questions.length} available questions viewed</span>
              ) : (
                <>
                  <span className="question-review-position">
                    Question {displayedActiveIndex + 1} of {questions.length}
                  </span>
                  <span className={`question-review-decision is-${activeQuestion.decision}`}>
                    {activeQuestion.decision}
                  </span>
                </>
              )}
            </div>
            <div className="question-review-pagination" aria-label="Question navigation">
              <button
                type="button"
                onClick={goToPreviousQuestion}
                disabled={displayedActiveIndex === 0 || isSaving}
                aria-label="Previous question"
                aria-keyshortcuts={getAriaKeyShortcut(commands.reviewPrevious)}
              >
                <ChevronLeft aria-hidden="true" />
              </button>
              <button
                type="button"
                onClick={goToNextQuestion}
                disabled={isCaughtUp || (!isGenerationActive && displayedActiveIndex === lastAvailableIndex) || isSaving}
                aria-label="Next question"
                aria-keyshortcuts={getAriaKeyShortcut(commands.reviewNext)}
              >
                <ChevronRight aria-hidden="true" />
              </button>
            </div>
          </div>

          {isCaughtUp ? (
            <div className="question-review-caught-up" role="status">
              <span className="question-review-caught-up-icon" aria-hidden="true">
                <Sparkles />
              </span>
              <div>
                <strong>You’re caught up</strong>
                <p>Generating the next question… it will appear here automatically.</p>
              </div>
            </div>
          ) : (
          <div className="question-review-form">
            <label className="question-review-field question-review-prompt">
              <span>Question</span>
              <textarea
                value={activeQuestion.question}
                onChange={(event) => updateField('question', event.target.value)}
                rows={3}
                disabled={isSaving}
              />
            </label>

            <div className="question-review-options">
              {answerOptions.map((answer) => {
                const field = `option_${answer.toLowerCase()}` as EditableQuestionField
                return (
                  <label className="question-review-field" key={answer}>
                    <span>Option {answer}</span>
                    <textarea
                      value={activeQuestion[field]}
                      onChange={(event) => updateField(field, event.target.value)}
                      rows={2}
                      disabled={isSaving}
                    />
                  </label>
                )
              })}
            </div>

            <fieldset className="question-review-answer">
              <legend>Correct answer</legend>
              <div>
                {answerOptions.map((answer) => (
                  <label key={answer}>
                    <input
                      type="radio"
                      name={`correct-answer-${activeQuestion.reviewId}`}
                      value={answer}
                      checked={activeQuestion.correct_answer === answer}
                      onChange={() => updateField('correct_answer', answer)}
                      disabled={isSaving}
                    />
                    <span>{answer}</span>
                  </label>
                ))}
              </div>
            </fieldset>

            {activeQuestion.explanation && (
              <div className="question-review-explanation">
                <strong>Generated explanation</strong>
                <p>{activeQuestion.explanation}</p>
              </div>
            )}
          </div>
          )}

          {validationMessage && (
            <p className="question-review-validation" role="alert">
              {validationMessage}
            </p>
          )}

          <div className="question-review-shortcuts" aria-label="Question review keyboard shortcuts">
            {isCaughtUp ? (
              <span>
                <kbd>{previousShortcut?.display}</kbd>
                Back to last question
              </span>
            ) : (
              <>
                <span>
                  <kbd>{previousShortcut?.display}</kbd>
                  <kbd>{nextShortcut?.display}</kbd>
                  Navigate
                </span>
                <span>
                  <kbd>{discardShortcut?.display}</kbd>
                  Discard
                </span>
                <span>
                  <kbd>{keepShortcut?.display}</kbd>
                  Keep
                </span>
              </>
            )}
          </div>

          {!isCaughtUp && (
            <footer className="question-review-actions">
              <button
                type="button"
                className={`btn-secondary question-review-discard${activeQuestion.decision === 'discarded' ? ' is-active' : ''}`}
                onClick={() => decideQuestion('discarded')}
                disabled={isSaving}
                aria-pressed={activeQuestion.decision === 'discarded'}
                aria-keyshortcuts={getAriaKeyShortcut(commands.reviewDiscard)}
              >
                <X aria-hidden="true" />
                Discard
              </button>
              <button
                type="button"
                className={`btn-secondary question-review-keep${activeQuestion.decision === 'kept' ? ' is-active' : ''}`}
                onClick={() => decideQuestion('kept')}
                disabled={isSaving}
                aria-pressed={activeQuestion.decision === 'kept'}
                aria-keyshortcuts={getAriaKeyShortcut(commands.reviewKeep)}
              >
                <Check aria-hidden="true" />
                Keep
              </button>
            </footer>
          )}
        </div>
      </section>
    </dialog>
  )
}
