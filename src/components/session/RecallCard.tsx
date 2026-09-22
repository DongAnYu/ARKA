import { invoke } from '@tauri-apps/api/core'
import { Keyboard } from 'lucide-react'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { commands } from '../../commands/commands'
import { createReviewController, reviewIntervalLabel, type RecallItem, type RecallResult } from '../../learning-items/recall'
import type { LearningItem, ReviewResponse } from '../../learning-items/types'
import { getAriaKeyShortcut, getShortcut } from '../../shortcuts/defaultShortcuts'
import { useKeyboardShortcuts } from '../../shortcuts/useKeyboardShortcuts'
import { ExplanationPanel } from './ExplanationPanel'
import { FlashcardCard } from './FlashcardCard'
import { QuestionCard } from './QuestionCard'
import { flashcardRatings } from './recallOptions'

export function RecallCard({ item, planDate, spaceId, isExtra, isLast, onReviewed, onNext, onBusy }: {
  item: RecallItem
  planDate: string
  spaceId: number | null
  isExtra: boolean
  isLast: boolean
  onReviewed: (result: RecallResult) => void
  onNext: () => void
  onBusy: (busy: boolean) => void
}) {
  const [controller] = useState(() => createReviewController(item,
    (submission) => invoke<LearningItem>('review_study_item', {
      planDate,
      spaceId,
      isExtra,
      submission,
    })))
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot)
  const [selectedOptionId, setSelectedOptionId] = useState<string | null>(null)
  const [motionEnabled, setMotionEnabled] = useState(true)
  const focusTarget = useRef<HTMLDivElement>(null)
  const nextButton = useRef<HTMLButtonElement>(null)
  const continued = useRef(false)
  const isSubmitting = state.status === 'submitting'
  const isReviewed = state.status === 'reviewed'
  const submittedOptionId = state.response?.format === 'mcq' ? state.response.selected_option_id : null

  useEffect(() => { focusTarget.current?.focus() }, [])
  useEffect(() => {
    if (!isReviewed) return
    nextButton.current?.focus({ preventScroll: true })
    nextButton.current?.scrollIntoView({ block: 'nearest', behavior: 'instant' })
  }, [isReviewed])

  const submit = async (response: ReviewResponse) => {
    // Ignore duplicate events without releasing the parent's busy guard.
    if (controller.getSnapshot().status !== 'idle') return
    onBusy(true)
    const result = await controller.submit(response)
    if (result) onReviewed(result)
    onBusy(false)
  }

  const continueSession = () => {
    if (controller.getSnapshot().status !== 'reviewed' || continued.current) return false
    continued.current = true
    onNext()
    return true
  }

  const chooseByIndex = (choiceIndex: number) => {
    const currentState = controller.getSnapshot()
    if (currentState.status !== 'idle') return false

    if (item.format === 'mcq') {
      const option = item.options[choiceIndex]
      if (!option) return false
      setSelectedOptionId(option.id)
      return true
    }

    if (!currentState.revealed) return false
    const rating = flashcardRatings[choiceIndex]?.value
    if (!rating) return false
    void submit({ format: 'flashcard', rating })
    return true
  }

  const runPrimaryAction = () => {
    const currentState = controller.getSnapshot()
    if (currentState.status === 'submitting') return false
    if (currentState.status === 'reviewed') return continueSession()

    if (item.format === 'flashcard') {
      if (currentState.revealed) return false
      controller.reveal()
      return true
    }

    if (selectedOptionId === null) return false
    void submit({ format: 'mcq', selected_option_id: selectedOptionId })
    return true
  }

  useKeyboardShortcuts({
    scope: 'recall-session',
    handlers: {
      [commands.recallChoose1]: () => chooseByIndex(0),
      [commands.recallChoose2]: () => chooseByIndex(1),
      [commands.recallChoose3]: () => chooseByIndex(2),
      [commands.recallChoose4]: () => chooseByIndex(3),
      [commands.recallPrimaryAction]: runPrimaryAction,
    },
  })

  const choiceShortcutLabel = `${getShortcut(commands.recallChoose1)?.display}–${getShortcut(commands.recallChoose4)?.display}`
  const showChoiceShortcut = !isSubmitting && !isReviewed && (item.format === 'mcq' || state.revealed)
  const primaryShortcutLabel = isSubmitting
    ? null
    : isReviewed
      ? isLast ? 'Finish session' : 'Next item'
      : item.format === 'mcq'
        ? 'Submit answer'
        : state.revealed ? null : 'Reveal answer'

  return (
    <div className="session-recall-card" ref={focusTarget} tabIndex={-1}
      data-motion={motionEnabled ? 'animate' : 'instant'}
      onClickCapture={(event) => setMotionEnabled(event.detail > 0)}
      aria-label={`${item.format === 'mcq' ? 'Multiple choice' : 'Flashcard'}: ${item.prompt}`}>
      {state.error ? <div className="error-banner" role="alert">{state.error}</div> : null}
      {item.format === 'mcq' ? (
        <QuestionCard question={item} selectedOptionId={selectedOptionId}
          isSubmitted={isReviewed} isSubmitting={isSubmitting} onSelectOption={setSelectedOptionId}
          onSubmit={() => { if (selectedOptionId !== null) void submit({ format: 'mcq', selected_option_id: selectedOptionId }) }} />
      ) : (
        <FlashcardCard item={item} revealed={state.revealed} disabled={isSubmitting || isReviewed}
          rating={state.response?.format === 'flashcard' ? state.response.rating : null}
          onReveal={controller.reveal} onRate={(rating) => { void submit({ format: 'flashcard', rating }) }} />
      )}
      <div className="session-shortcuts" aria-label="Keyboard shortcuts">
        <span className="session-shortcuts-title"><Keyboard aria-hidden="true" />Keyboard</span>
        {showChoiceShortcut ? (
          <span><kbd className="session-keycap">{choiceShortcutLabel}</kbd>{item.format === 'mcq' ? 'Choose answer' : 'Rate recall'}</span>
        ) : null}
        {primaryShortcutLabel ? (
          <span><kbd className="session-keycap">{getShortcut(commands.recallPrimaryAction)?.display}</kbd>{primaryShortcutLabel}</span>
        ) : null}
      </div>
      {isSubmitting || (isReviewed && state.nextReviewDays !== null) ? (
        <div className="session-review-status" role="status">
          {isSubmitting ? 'Saving review…' : `Review saved. Next review in ${reviewIntervalLabel(state.nextReviewDays ?? 0)}.`}
        </div>
      ) : null}
      {isReviewed && item.format === 'mcq' && state.response?.format === 'mcq' ? (
        <ExplanationPanel isCorrect={state.response.selected_option_id === item.correct_option_id}
          selectedAnswer={item.options.find((option) => option.id === submittedOptionId)?.text ?? ''}
          correctAnswer={item.options.find((option) => option.id === item.correct_option_id)?.text ?? ''}
          explanation={item.explanation} />
      ) : null}
      {isReviewed ? (
        <div className="session-navigation">
          <button ref={nextButton} type="button" className="btn-primary session-next-btn"
            onClick={continueSession} aria-keyshortcuts={getAriaKeyShortcut(commands.recallPrimaryAction)}>
            <span>{isLast ? 'Finish session' : 'Next item'}</span>
            <kbd className="session-keycap session-action-key" aria-hidden="true">
              {getShortcut(commands.recallPrimaryAction)?.display}
            </kbd>
          </button>
        </div>
      ) : null}
    </div>
  )
}
