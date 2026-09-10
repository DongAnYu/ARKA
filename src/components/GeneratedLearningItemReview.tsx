import { useEffect, useRef, useState } from 'react'
import {
  ArrowLeft,
  ArrowRight,
  Check,
  CheckCheck,
  ChevronDown,
  CreditCard,
  FolderOpen,
  Info,
  ListChecks,
  Sparkles,
  X,
} from 'lucide-react'
import {
  getReviewLearningItemError,
  type ReviewDecision,
  type ReviewLearningItemDraft,
} from '../generation/review'
import type { GeneratedItem } from '../generation/types'
import { commands } from '../commands/commands'
import { getAriaKeyShortcut, getShortcut } from '../shortcuts/defaultShortcuts'
import { useKeyboardShortcuts } from '../shortcuts/useKeyboardShortcuts'

type RecallSpaceOption = {
  id: number
  name: string
}

type GeneratedLearningItemReviewProps = {
  items: ReviewLearningItemDraft[]
  modelName: string
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
  onUpdateItem: (
    draftId: string,
    updates: Partial<ReviewLearningItemDraft>,
  ) => void
  onKeepRemaining: () => void
  onSaveKept: () => void
  onFinishWithoutSaving: () => void
  onClose: (activeIndex: number) => void
}

export function GeneratedLearningItemReview({
  items,
  modelName,
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
  onUpdateItem,
  onKeepRemaining,
  onSaveKept,
  onFinishWithoutSaving,
  onClose,
}: GeneratedLearningItemReviewProps) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const maxInitialIndex = isGenerationActive
    ? items.length
    : Math.max(items.length - 1, 0)
  const [activeIndex, setActiveIndex] = useState(
    Math.min(Math.max(initialActiveIndex, 0), maxInitialIndex),
  )
  const [validationMessage, setValidationMessage] = useState('')

  const lastAvailableIndex = Math.max(items.length - 1, 0)
  const displayedActiveIndex = isGenerationActive
    ? activeIndex
    : Math.min(activeIndex, lastAvailableIndex)
  const isCaughtUp = isGenerationActive && activeIndex >= items.length
  const activeItem = items[Math.min(displayedActiveIndex, lastAvailableIndex)]
  const keptCount = items.filter((item) => item.decision === 'kept').length
  const discardedCount = items.filter(
    (item) => item.decision === 'discarded',
  ).length
  const remainingCount = items.length - keptCount - discardedCount
  const decidedCount = items.length - remainingCount
  const reviewPercent = items.length
    ? Math.round((decidedCount / items.length) * 100)
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

  const updateContent = (updates: Partial<GeneratedItem>) => {
    if (!activeItem || isCaughtUp) {
      return
    }

    setValidationMessage('')
    onUpdateItem(activeItem.draft_id, {
      content: {
        ...activeItem.content,
        ...updates,
      },
    })
  }

  const decideItem = (decision: Exclude<ReviewDecision, 'pending'>) => {
    if (!activeItem || isCaughtUp) {
      return false
    }

    const nextDecision: ReviewDecision =
      activeItem.decision === decision ? 'pending' : decision

    if (nextDecision === 'kept') {
      const message = getReviewLearningItemError(activeItem)
      if (message) {
        setValidationMessage(message)
        return false
      }
    }

    onUpdateItem(activeItem.draft_id, { decision: nextDecision })
    setValidationMessage('')
    return true
  }

  const keepRemaining = () => {
    const invalidIndex = items.findIndex(
      (item) =>
        item.decision === 'pending' && Boolean(getReviewLearningItemError(item)),
    )

    if (invalidIndex !== -1) {
      setActiveIndex(invalidIndex)
      setValidationMessage(getReviewLearningItemError(items[invalidIndex]))
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

  const goToPreviousItem = () => {
    if (displayedActiveIndex === 0 || isSaving) {
      return false
    }

    setActiveIndex(Math.max(0, displayedActiveIndex - 1))
    setValidationMessage('')
    return true
  }

  const goToNextItem = () => {
    if (isSaving || isCaughtUp) {
      return false
    }

    if (displayedActiveIndex === lastAvailableIndex) {
      if (!isGenerationActive) {
        return false
      }

      setActiveIndex(items.length)
      setValidationMessage('')
      return true
    }

    setActiveIndex(Math.min(lastAvailableIndex, displayedActiveIndex + 1))
    setValidationMessage('')
    return true
  }

  useKeyboardShortcuts({
    scope: 'question-review',
    enabled: Boolean(activeItem) && !isSaving,
    handlers: {
      [commands.reviewPrevious]: goToPreviousItem,
      [commands.reviewNext]: goToNextItem,
      [commands.reviewDiscard]: () => decideItem('discarded'),
      [commands.reviewKeep]: () => decideItem('kept'),
    },
  })

  if (!activeItem) {
    return null
  }

  const previousShortcut = getShortcut(commands.reviewPrevious)
  const nextShortcut = getShortcut(commands.reviewNext)
  const discardShortcut = getShortcut(commands.reviewDiscard)
  const keepShortcut = getShortcut(commands.reviewKeep)
  const mcq = activeItem.content.mcq
  const distractors = mcq?.distractors ?? []

  return (
    <dialog
      ref={dialogRef}
      className="question-review-dialog learning-item-review-dialog"
      aria-labelledby="learning-item-review-title"
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
      <section className="question-review learning-item-review">
        <header className="question-review-header learning-item-review-header">
          <div className="question-review-title-group">
            <div className="learning-item-review-heading">
              <h3 id="learning-item-review-title">Learning item review</h3>
              <p>
                {isGenerationActive
                  ? 'Refine complete drafts while ARKA keeps generating in the background.'
                  : 'Refine each draft before anything is added to your library.'}
              </p>
              <div
                className="learning-item-review-model"
                aria-label={`Generation model: ${modelName}`}
                title={`LLM: ${modelName}`}
              >
                <span className="learning-item-review-model-label">LLM</span>
                <span className="learning-item-review-model-value">{modelName}</span>
              </div>
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
                  'Adding items…'
                ) : (
                  <>
                    Add {keptCount} {keptCount === 1 ? 'Learning Item' : 'Learning Items'}
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
              aria-label="Close learning item review"
            >
              <X aria-hidden="true" />
            </button>
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
                        <option key={space.id} value={space.id}>{space.name}</option>
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
                  ? 'Name the new Recall Space before adding learning items.'
                  : 'Choose an existing Recall Space or create a new one.'}
              </p>
            )}
          </div>
        </header>

        <div className="learning-item-review-progress-row">
          <strong>
            {isCaughtUp
              ? `All ${items.length} available items viewed`
              : `Item ${displayedActiveIndex + 1} of ${items.length}`}
          </strong>
          <div className="question-review-progress" aria-hidden="true">
            <span style={{ transform: `scaleX(${reviewPercent / 100})` }} />
          </div>
          <span>{remainingCount} remaining</span>
        </div>

        <div className="question-review-workspace learning-item-review-workspace">
          {isCaughtUp ? (
            <div className="question-review-caught-up" role="status">
              <span className="question-review-caught-up-icon" aria-hidden="true">
                <Sparkles />
              </span>
              <div>
                <strong>You’re caught up</strong>
                <p>Generating the next learning item… it will appear here automatically.</p>
              </div>
            </div>
          ) : (
            <div className="learning-item-review-columns">
              <section className="learning-item-review-panel" aria-labelledby="core-knowledge-title">
                <div className="learning-item-review-section-heading">
                  <h4 id="core-knowledge-title">Core knowledge</h4>
                  <p>Shared content for every practice variant in this learning item.</p>
                </div>

                <div className="learning-item-review-fields">
                  <label className="question-review-field">
                    <span>Learning target</span>
                    <textarea
                      className="edit-space-input edit-space-textarea"
                      value={activeItem.content.target}
                      onChange={(event) => updateContent({ target: event.target.value })}
                      rows={3}
                      disabled={isSaving}
                    />
                  </label>

                  <label className="question-review-field">
                    <span>Answer</span>
                    <textarea
                      className="edit-space-input edit-space-textarea"
                      value={activeItem.content.answer}
                      onChange={(event) => updateContent({ answer: event.target.value })}
                      rows={4}
                      disabled={isSaving}
                    />
                  </label>

                  <label className="question-review-field">
                    <span>Explanation <small>(optional)</small></span>
                    <textarea
                      className="edit-space-input edit-space-textarea"
                      value={activeItem.content.explanation ?? ''}
                      onChange={(event) => updateContent({
                        explanation: event.target.value || null,
                      })}
                      rows={4}
                      disabled={isSaving}
                    />
                  </label>
                </div>
              </section>

              <section className="learning-item-review-panel" aria-labelledby="practice-variants-title">
                <div className="learning-item-review-section-heading">
                  <h4 id="practice-variants-title">Practice variants</h4>
                  <p>Different formats for active recall, using the same core knowledge.</p>
                </div>

                <article className="learning-item-variant-card">
                  <div className="learning-item-variant-heading">
                    <CreditCard aria-hidden="true" />
                    <strong>Flashcard</strong>
                    <span className="learning-item-variant-badge is-required">Required</span>
                  </div>
                  <label className="question-review-field">
                    <span>Prompt</span>
                    <textarea
                      className="edit-space-input edit-space-textarea"
                      value={activeItem.content.flashcard.prompt}
                      onChange={(event) => updateContent({
                        flashcard: { prompt: event.target.value },
                      })}
                      rows={2}
                      disabled={isSaving}
                    />
                  </label>
                  <p className="learning-item-shared-answer-note">
                    <Info aria-hidden="true" />
                    Uses the shared answer on the left. The answer is not duplicated here.
                  </p>
                </article>

                {mcq ? (
                  <article className="learning-item-variant-card">
                    <div className="learning-item-variant-heading">
                      <ListChecks aria-hidden="true" />
                      <strong>Multiple choice</strong>
                      <span className="learning-item-variant-badge">Optional</span>
                    </div>
                    <label className="question-review-field">
                      <span>Question</span>
                      <textarea
                        className="edit-space-input edit-space-textarea"
                        value={mcq.prompt ?? activeItem.content.flashcard.prompt}
                        onChange={(event) => updateContent({
                          mcq: { ...mcq, prompt: event.target.value },
                        })}
                        rows={2}
                        disabled={isSaving}
                      />
                    </label>
                    <fieldset className="learning-item-mcq-options">
                      <legend>Answer options</legend>
                      <div className="is-correct">
                        <input
                          type="radio"
                          name={`correct-answer-${activeItem.draft_id}`}
                          checked
                          readOnly
                          aria-label="Shared answer is correct"
                        />
                        <span>Correct</span>
                        <p title="Edit this value in the shared Answer field">
                          {activeItem.content.answer}
                        </p>
                      </div>
                      {Array.from({ length: 3 }, (_, distractorIndex) => (
                        <label key={`${activeItem.draft_id}-distractor-${distractorIndex}`}>
                          <input
                            type="radio"
                            name={`correct-answer-${activeItem.draft_id}`}
                            checked={false}
                            onChange={() => {
                              const nextAnswer = distractors[distractorIndex] ?? ''
                              const nextDistractors = [...distractors]
                              nextDistractors[distractorIndex] = activeItem.content.answer
                              updateContent({
                                answer: nextAnswer,
                                mcq: { ...mcq, distractors: nextDistractors },
                              })
                            }}
                            aria-label={`Make distractor ${distractorIndex + 1} the correct answer`}
                            disabled={isSaving}
                          />
                          <span>{distractorIndex + 1}</span>
                          <textarea
                            className="edit-space-input edit-question-option-textarea"
                            value={distractors[distractorIndex] ?? ''}
                            onChange={(event) => {
                              const nextDistractors = [...distractors]
                              nextDistractors[distractorIndex] = event.target.value
                              updateContent({
                                mcq: { ...mcq, distractors: nextDistractors },
                              })
                            }}
                            rows={1}
                            aria-label={`Distractor ${distractorIndex + 1}`}
                            disabled={isSaving}
                          />
                        </label>
                      ))}
                      <small>The answer order is mixed when the item is saved.</small>
                    </fieldset>
                  </article>
                ) : (
                  <article className="learning-item-variant-card is-omitted">
                    <div className="learning-item-variant-heading">
                      <X aria-hidden="true" />
                      <strong>Multiple choice omitted</strong>
                    </div>
                    <label className="question-review-field">
                      <span>Reason</span>
                      <textarea
                        className="edit-space-input edit-space-textarea"
                        value={activeItem.content.mcq_omission_reason ?? ''}
                        onChange={(event) => updateContent({
                          mcq_omission_reason: event.target.value,
                        })}
                        rows={3}
                        disabled={isSaving}
                      />
                    </label>
                  </article>
                )}
              </section>
            </div>
          )}

          {validationMessage && (
            <p className="question-review-validation" role="alert">
              {validationMessage}
            </p>
          )}
        </div>

        <footer className="learning-item-review-footer">
          <div className="learning-item-review-pagination" aria-label="Learning item navigation">
            <button
              type="button"
              className="btn-secondary"
              onClick={goToPreviousItem}
              disabled={displayedActiveIndex === 0 || isSaving}
              aria-keyshortcuts={getAriaKeyShortcut(commands.reviewPrevious)}
            >
              <ArrowLeft aria-hidden="true" />
              Previous
            </button>
            <button
              type="button"
              className="btn-secondary"
              onClick={goToNextItem}
              disabled={isCaughtUp || (!isGenerationActive && displayedActiveIndex === lastAvailableIndex) || isSaving}
              aria-keyshortcuts={getAriaKeyShortcut(commands.reviewNext)}
            >
              <ArrowRight aria-hidden="true" />
              Next
            </button>
          </div>

          <div className="question-review-shortcuts" aria-label="Learning item review keyboard shortcuts">
            <span><kbd>{previousShortcut?.display}</kbd><kbd>{nextShortcut?.display}</kbd>Navigate</span>
            <span><kbd>{discardShortcut?.display}</kbd>Discard</span>
            <span><kbd>{keepShortcut?.display}</kbd>Keep</span>
          </div>

          {!isCaughtUp && (
            <div className="learning-item-review-decisions">
              <button
                type="button"
                className={`btn-secondary question-review-discard${activeItem.decision === 'discarded' ? ' is-active' : ''}`}
                onClick={() => decideItem('discarded')}
                disabled={isSaving}
                aria-pressed={activeItem.decision === 'discarded'}
                aria-keyshortcuts={getAriaKeyShortcut(commands.reviewDiscard)}
              >
                <X aria-hidden="true" />
                Discard
              </button>
              <button
                type="button"
                className={`btn-primary question-review-keep${activeItem.decision === 'kept' ? ' is-active' : ''}`}
                onClick={() => decideItem('kept')}
                disabled={isSaving}
                aria-pressed={activeItem.decision === 'kept'}
                aria-keyshortcuts={getAriaKeyShortcut(commands.reviewKeep)}
              >
                <Check aria-hidden="true" />
                Keep
              </button>
            </div>
          )}
        </footer>
      </section>
    </dialog>
  )
}
