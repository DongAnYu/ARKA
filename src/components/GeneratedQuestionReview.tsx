import { useEffect, useRef, useState } from 'react'
import {
  ArrowRight,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  X,
} from 'lucide-react'
import {
  answerOptions,
  getReviewQuestionError,
  type ReviewDecision,
  type ReviewQuestionDraft,
} from '../generation/review'

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
  initialActiveIndex,
  onUpdateQuestion,
  onKeepRemaining,
  onSaveKept,
  onFinishWithoutSaving,
  onClose,
}: GeneratedQuestionReviewProps) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const [activeIndex, setActiveIndex] = useState(
    Math.min(Math.max(initialActiveIndex, 0), Math.max(questions.length - 1, 0)),
  )
  const [validationMessage, setValidationMessage] = useState('')

  const activeQuestion = questions[activeIndex]
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

  if (!activeQuestion) {
    return null
  }

  const updateField = (field: EditableQuestionField, value: string) => {
    setValidationMessage('')
    onUpdateQuestion(activeQuestion.reviewId, { [field]: value })
  }

  const decideQuestion = (decision: Exclude<ReviewDecision, 'pending'>) => {
    const nextDecision: ReviewDecision =
      activeQuestion.decision === decision ? 'pending' : decision

    if (nextDecision === 'kept') {
      const message = getReviewQuestionError(activeQuestion)
      if (message) {
        setValidationMessage(message)
        return
      }
    }

    onUpdateQuestion(activeQuestion.reviewId, { decision: nextDecision })
    setValidationMessage('')
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
      onClose(activeIndex)
    }
  }

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
              <p>Refine each draft before anything is added to your library.</p>
            </div>
            <div className="question-review-counts" aria-live="polite">
              <span className="is-kept"><strong>{keptCount}</strong> kept</span>
              <span className="is-discarded"><strong>{discardedCount}</strong> discarded</span>
              <span className="is-remaining"><strong>{remainingCount}</strong> remaining</span>
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
              <span className="question-review-position">
                Question {activeIndex + 1} of {questions.length}
              </span>
              <span className={`question-review-decision is-${activeQuestion.decision}`}>
                {activeQuestion.decision}
              </span>
            </div>
            <div className="question-review-pagination" aria-label="Question navigation">
              <button
                type="button"
                onClick={() => {
                  setActiveIndex((index) => Math.max(0, index - 1))
                  setValidationMessage('')
                }}
                disabled={activeIndex === 0 || isSaving}
                aria-label="Previous question"
              >
                <ChevronLeft aria-hidden="true" />
              </button>
              <button
                type="button"
                onClick={() => {
                  setActiveIndex((index) => Math.min(questions.length - 1, index + 1))
                  setValidationMessage('')
                }}
                disabled={activeIndex === questions.length - 1 || isSaving}
                aria-label="Next question"
              >
                <ChevronRight aria-hidden="true" />
              </button>
            </div>
          </div>

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

          {validationMessage && (
            <p className="question-review-validation" role="alert">
              {validationMessage}
            </p>
          )}

          <footer className="question-review-actions">
            <button
              type="button"
              className={`btn-secondary question-review-discard${activeQuestion.decision === 'discarded' ? ' is-active' : ''}`}
              onClick={() => decideQuestion('discarded')}
              disabled={isSaving}
              aria-pressed={activeQuestion.decision === 'discarded'}
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
            >
              <Check aria-hidden="true" />
              Keep
            </button>
          </footer>
        </div>
      </section>
    </dialog>
  )
}
