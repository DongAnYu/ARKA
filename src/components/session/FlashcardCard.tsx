import { invoke } from '@tauri-apps/api/core'
import { useEffect, useRef, useState } from 'react'
import { Check, RotateCcw } from 'lucide-react'
import { commands, recallChoiceCommands } from '../../commands/commands'
import { reviewIntervalLabel, type RecallItem } from '../../learning-items/recall'
import type { ReviewIntervals, ReviewRating } from '../../learning-items/types'
import { getAriaKeyShortcut, getShortcut } from '../../shortcuts/defaultShortcuts'
import { GenerationModel } from './GenerationModel'
import { flashcardRatings } from './recallOptions'

export function FlashcardCard({ item, revealed, disabled, rating, onReveal, onRate }: {
  item: Extract<RecallItem, { format: 'flashcard' }>
  revealed: boolean
  disabled: boolean
  rating: ReviewRating | null
  onReveal: () => void
  onRate: (rating: ReviewRating) => void
}) {
  const answer = useRef<HTMLDivElement>(null)
  const [intervals, setIntervals] = useState<ReviewIntervals | null>(null)
  const [intervalError, setIntervalError] = useState(false)
  const [previewAttempt, setPreviewAttempt] = useState(0)
  const outcome = rating ? rating === 'again' ? 'again' : 'recalled' : undefined
  useEffect(() => {
    let cancelled = false
    void invoke<ReviewIntervals>('get_learning_item_review_intervals', { learningItemId: item.learningItemId })
      .then((result) => { if (!cancelled) setIntervals(result) })
      .catch(() => { if (!cancelled) setIntervalError(true) })
    return () => { cancelled = true }
  }, [item.learningItemId, previewAttempt])
  useEffect(() => { if (revealed) answer.current?.focus() }, [revealed])
  return (
    <section className="session-question-card session-flashcard surface-panel" aria-label="Flashcard" data-outcome={outcome}>
      <div className="session-question-head">
        <h2>{item.prompt}</h2>
        <GenerationModel model={item.model} />
      </div>
      {!revealed ? (
        <div className="session-question-actions">
          <button type="button" className="btn-primary session-reveal-btn" onClick={onReveal}
            aria-keyshortcuts={getAriaKeyShortcut(commands.recallPrimaryAction)}>
            <span>Reveal answer</span>
            <kbd className="session-keycap session-action-key" aria-hidden="true">
              {getShortcut(commands.recallPrimaryAction)?.display}
            </kbd>
          </button>
        </div>
      ) : (
        <>
          <div ref={answer} className="session-flashcard-answer" tabIndex={-1} aria-label="Revealed answer" aria-live="polite">
            <h3>Answer</h3>
            <p>{item.answer}</p>
            {item.explanation ? <><h3>Explanation</h3><p>{item.explanation}</p></> : null}
          </div>
          {rating ? (
            <div className="session-rating-result" role="status">
              <span className="session-rating-result-icon" aria-hidden="true">
                {rating === 'again' ? <RotateCcw /> : <Check />}
              </span>
              <div>
                <strong>{rating === 'again' ? 'Again' : 'Recalled'}</strong>
                <p>{flashcardRatings.find((entry) => entry.value === rating)?.label} · Review saved</p>
              </div>
            </div>
          ) : null}
          <fieldset className="session-rating-controls" disabled={disabled}>
            <legend>How well did you recall this?</legend>
            <div className="session-rating-grid">
              {flashcardRatings.map(({ value, label }, index) => {
                const shortcutCommand = recallChoiceCommands[index]
                const shortcut = getShortcut(shortcutCommand)
                return (
                <button key={value} type="button" className="btn-secondary" data-rating={value}
                  aria-pressed={rating === value} aria-keyshortcuts={getAriaKeyShortcut(shortcutCommand)}
                  onClick={() => onRate(value)}>
                  <span className="session-rating-main">
                    <span>{label}</span>
                    <kbd className="session-keycap session-rating-key" aria-hidden="true">{shortcut?.display}</kbd>
                  </span>
                  <span className="session-rating-interval">
                    {intervals ? `+${reviewIntervalLabel(intervals[value])}` : intervalError ? 'Interval unavailable' : 'Calculating…'}
                  </span>
                </button>
                )
              })}
            </div>
          </fieldset>
          {intervalError && !rating ? (
            <div className="session-interval-error" role="status">
              <span>Intervals could not be loaded. You can still rate this item.</span>
              <button type="button" className="btn-secondary" disabled={disabled} onClick={() => {
                setIntervalError(false)
                setPreviewAttempt((attempt) => attempt + 1)
              }}>Retry intervals</button>
            </div>
          ) : null}
        </>
      )}
    </section>
  )
}
