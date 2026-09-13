import { invoke } from '@tauri-apps/api/core'
import { useEffect, useRef, useState } from 'react'
import { Check, RotateCcw } from 'lucide-react'
import { reviewIntervalLabel, type RecallItem } from '../../learning-items/recall'
import type { ReviewIntervals, ReviewRating } from '../../learning-items/types'
import { GenerationModel } from './GenerationModel'

const ratings: { value: ReviewRating; label: string }[] = [
  { value: 'again', label: 'Again' },
  { value: 'hard', label: 'Hard' },
  { value: 'good', label: 'Good' },
  { value: 'easy', label: 'Easy' },
]

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
          <button type="button" className="btn-primary" onClick={onReveal}>Reveal answer</button>
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
                <p>{ratings.find((entry) => entry.value === rating)?.label} · Review saved</p>
              </div>
            </div>
          ) : null}
          <fieldset className="session-rating-controls" disabled={disabled}>
            <legend>How well did you recall this?</legend>
            <div className="session-rating-grid">
              {ratings.map(({ value, label }) => (
                <button key={value} type="button" className="btn-secondary" data-rating={value}
                  aria-pressed={rating === value} onClick={() => onRate(value)}>
                  <span>{label}</span>
                  <span className="session-rating-interval">
                    {intervals ? `+${reviewIntervalLabel(intervals[value])}` : intervalError ? 'Interval unavailable' : 'Calculating…'}
                  </span>
                </button>
              ))}
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
