import { invoke } from '@tauri-apps/api/core'
import { ChevronDown } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ExplanationPanel } from '../components/session/ExplanationPanel'
import { QuestionCard, type OptionId, type SessionQuestion } from '../components/session/QuestionCard'
import { SessionComplete } from '../components/session/SessionComplete'
import { SessionHeader } from '../components/session/SessionHeader'

type SchedulerRating = 'again' | 'easy'

type SessionReview = {
  questionId: number
  selectedOptionId: OptionId
  isCorrect: boolean
}

type RecallSpace = {
  id: number
  name: string
  description: string | null
}

type StoredQuestion = {
  id: number
  question: string
  option_a: string
  option_b: string
  option_c: string
  option_d: string
  correct_answer: OptionId
  explanation: string | null
  space_id: number
}

type SpaceFilterValue = 'all' | number

const ALL_SPACES_VALUE = 'all'

const getSchedulerRating = (isCorrect: boolean): SchedulerRating => (isCorrect ? 'easy' : 'again')

const formatDuration = (elapsedMs: number): string => {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000))
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60

  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`
}

async function reviewQuestion(questionId: number, rating: SchedulerRating): Promise<void> {
  await invoke('review_question', { questionId, rating })
}

const toSessionQuestion = (question: StoredQuestion): SessionQuestion => ({
  id: question.id,
  prompt: question.question,
  options: [
    { id: 'A', text: question.option_a },
    { id: 'B', text: question.option_b },
    { id: 'C', text: question.option_c },
    { id: 'D', text: question.option_d },
  ],
  correctOptionId: question.correct_answer,
  explanation: question.explanation ?? undefined,
})

export function SessionPage() {
  const [spaces, setSpaces] = useState<RecallSpace[]>([])
  const [selectedSpaceId, setSelectedSpaceId] = useState<SpaceFilterValue>(ALL_SPACES_VALUE)
  const [dueQuestions, setDueQuestions] = useState<SessionQuestion[]>([])
  const [currentIndex, setCurrentIndex] = useState(0)
  const [selectedOptionId, setSelectedOptionId] = useState<OptionId | null>(null)
  const [isSubmitted, setIsSubmitted] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState('')
  const [reviews, setReviews] = useState<SessionReview[]>([])
  const [sessionStartedAt, setSessionStartedAt] = useState<number | null>(null)
  const [sessionCompletedAt, setSessionCompletedAt] = useState<number | null>(null)

  const totalQuestions = dueQuestions.length
  const isComplete = currentIndex >= totalQuestions
  const currentQuestion = isComplete ? null : dueQuestions[currentIndex]

  const selectedSpaceName = useMemo(() => {
    if (selectedSpaceId === ALL_SPACES_VALUE) {
      return 'All Recall Spaces'
    }

    return spaces.find((space) => space.id === selectedSpaceId)?.name ?? 'Recall Space'
  }, [selectedSpaceId, spaces])

  const currentReview = useMemo(() => {
    if (!currentQuestion) {
      return null
    }

    return reviews.find((review) => review.questionId === currentQuestion.id) ?? null
  }, [currentQuestion, reviews])

  const correctCount = useMemo(
    () => reviews.reduce((count, review) => count + (review.isCorrect ? 1 : 0), 0),
    [reviews],
  )

  const sessionDurationLabel = useMemo(() => {
    if (sessionCompletedAt === null || sessionStartedAt === null) {
      return '00:00'
    }

    return formatDuration(sessionCompletedAt - sessionStartedAt)
  }, [sessionCompletedAt, sessionStartedAt])

  const applySessionQuestions = (questions: StoredQuestion[]) => {
    setDueQuestions(questions.map(toSessionQuestion))
    setCurrentIndex(0)
    setSelectedOptionId(null)
    setIsSubmitted(false)
    setIsSubmitting(false)
    setReviews([])
    setSessionCompletedAt(null)
    setSessionStartedAt(questions.length > 0 ? Date.now() : null)
  }

  const fetchDueQuestions = async (spaceId: SpaceFilterValue): Promise<StoredQuestion[]> => {
    return invoke<StoredQuestion[]>('get_due_questions', {
      spaceId: spaceId === ALL_SPACES_VALUE ? null : spaceId,
    })
  }

  useEffect(() => {
    let isCancelled = false

    void (async () => {
      try {
        const [loadedSpaces, loadedQuestions] = await Promise.all([
          invoke<RecallSpace[]>('get_spaces'),
          fetchDueQuestions(ALL_SPACES_VALUE),
        ])

        if (isCancelled) {
          return
        }

        setSpaces(loadedSpaces)
        applySessionQuestions(loadedQuestions)
        setError('')
      } catch (err) {
        if (isCancelled) {
          return
        }

        const message = err instanceof Error ? err.message : 'Failed to load session questions'
        setError(message)
        applySessionQuestions([])
      } finally {
        if (!isCancelled) {
          setIsLoading(false)
        }
      }
    })()

    return () => {
      isCancelled = true
    }
  }, [])

  const handleSpaceChange = async (value: SpaceFilterValue) => {
    setSelectedSpaceId(value)
    setIsLoading(true)
    setError('')

    try {
      const loadedQuestions = await fetchDueQuestions(value)
      applySessionQuestions(loadedQuestions)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load session questions'
      setError(message)
      applySessionQuestions([])
    } finally {
      setIsLoading(false)
    }
  }

  const handleSubmit = async () => {
    if (!currentQuestion || !selectedOptionId || isSubmitted || isSubmitting) {
      return
    }

    const isCorrect = selectedOptionId === currentQuestion.correctOptionId
    const rating = getSchedulerRating(isCorrect)

    setIsSubmitting(true)
    setError('')

    try {
      await reviewQuestion(currentQuestion.id, rating)

      setReviews((current) => {
        const existingIndex = current.findIndex((item) => item.questionId === currentQuestion.id)
        const nextReview: SessionReview = {
          questionId: currentQuestion.id,
          selectedOptionId,
          isCorrect,
        }

        if (existingIndex === -1) {
          return [...current, nextReview]
        }

        const next = [...current]
        next[existingIndex] = nextReview
        return next
      })

      setIsSubmitted(true)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to review question'
      setError(message)
    } finally {
      setIsSubmitting(false)
    }
  }

  const handleNextQuestion = () => {
    if (!isSubmitted) {
      return
    }

    const nextIndex = currentIndex + 1

    if (nextIndex >= totalQuestions) {
      setSessionCompletedAt(Date.now())
    }

    setCurrentIndex(nextIndex)
    setSelectedOptionId(null)
    setIsSubmitted(false)
  }

  return (
    <div className="app-container session-page" aria-label="Session page">
      <section className="settings-panel" aria-label="Session filter">
        <div className="settings-section-head">
          <h3>Review queue</h3>
          <p>Load questions that are due now, optionally filtered by recall space.</p>
        </div>

        <div className="recall-space-select-wrap">
          <span>Recall Space</span>
          <select
            className="recall-space-select"
            value={selectedSpaceId === ALL_SPACES_VALUE ? ALL_SPACES_VALUE : String(selectedSpaceId)}
            onChange={(event) => {
              const value = event.target.value
              void handleSpaceChange(value === ALL_SPACES_VALUE ? ALL_SPACES_VALUE : Number(value))
            }}
            disabled={isLoading || isSubmitting}
          >
            <option value={ALL_SPACES_VALUE}>All recall spaces</option>
            {spaces.map((space) => (
              <option key={space.id} value={space.id}>
                {space.name}
              </option>
            ))}
          </select>
          <ChevronDown className="recall-space-chevron" aria-hidden="true" />
        </div>
      </section>

      {error ? <div className="error-banner">{error}</div> : null}

      {isLoading ? (
        <section className="settings-panel">
          <p className="settings-help-text">Loading questions due for review...</p>
        </section>
      ) : totalQuestions === 0 ? (
        <section className="settings-panel">
          <p className="settings-help-text">
            No questions are due right now{selectedSpaceId === ALL_SPACES_VALUE ? '.' : ` in ${selectedSpaceName}.`}
          </p>
        </section>
      ) : isComplete ? (
        <SessionComplete
          reviewedCount={reviews.length}
          correctCount={correctCount}
          durationLabel={sessionDurationLabel}
        />
      ) : currentQuestion ? (
        <div className="session-shell">
          <SessionHeader
            recallSpaceName={selectedSpaceName}
            currentQuestionNumber={currentIndex + 1}
            totalQuestions={totalQuestions}
          />

          <QuestionCard
            question={currentQuestion}
            selectedOptionId={selectedOptionId}
            isSubmitted={isSubmitted}
            isSubmitting={isSubmitting}
            onSelectOption={setSelectedOptionId}
            onSubmit={() => {
              void handleSubmit()
            }}
          />

          {isSubmitted && currentReview ? (
            <>
              <ExplanationPanel
                isCorrect={currentReview.isCorrect}
                selectedOptionId={currentReview.selectedOptionId}
                correctOptionId={currentQuestion.correctOptionId}
                explanation={currentQuestion.explanation ?? null}
              />

              <div className="session-navigation">
                <button type="button" className="btn-primary session-next-btn" onClick={handleNextQuestion}>
                  Next question
                </button>
              </div>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
