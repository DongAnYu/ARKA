import { invoke } from '@tauri-apps/api/core'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { createReviewController, reviewIntervalLabel, type RecallItem, type RecallResult } from '../../learning-items/recall'
import type { LearningItem, ReviewResponse } from '../../learning-items/types'
import { ExplanationPanel } from './ExplanationPanel'
import { FlashcardCard } from './FlashcardCard'
import { QuestionCard } from './QuestionCard'

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
      <div role="status">
        {isSubmitting ? 'Saving review…' : isReviewed && state.nextReviewDays !== null
          ? `Review saved. Next review in ${reviewIntervalLabel(state.nextReviewDays)}.` : ''}
      </div>
      {isReviewed && item.format === 'mcq' && state.response?.format === 'mcq' ? (
        <ExplanationPanel isCorrect={state.response.selected_option_id === item.correct_option_id}
          selectedAnswer={item.options.find((option) => option.id === submittedOptionId)?.text ?? ''}
          correctAnswer={item.options.find((option) => option.id === item.correct_option_id)?.text ?? ''}
          explanation={item.explanation} />
      ) : null}
      {isReviewed ? (
        <div className="session-navigation">
          <button ref={nextButton} type="button" className="btn-primary session-next-btn" onClick={() => {
            if (continued.current) return
            continued.current = true
            onNext()
          }}>{isLast ? 'Finish session' : 'Next item'}</button>
        </div>
      ) : null}
    </div>
  )
}
