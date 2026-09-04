import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowRight, BookOpenCheck, ChevronDown, RotateCcw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useLocation } from 'react-router-dom'
import { BackToHome } from '../components/BackToHome'
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

type RecallDashboard = {
  due_today_count: number
  overdue_count: number
  reviewed_today_count: number
  correct_today_count: number
  spaces: RecallSpaceSummary[]
}

type RecallSpaceSummary = {
  id: number
  name: string
  total_questions: number
  due_count: number
  overdue_count: number
  reviewed_today_count: number
  correct_today_count: number
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

const getSchedulerRating = (isCorrect: boolean): SchedulerRating => (isCorrect ? 'easy' : 'again')

const questionLabel = (count: number) => `${count} ${count === 1 ? 'question' : 'questions'}`

type RecallChartCategory = {
  label: string
  value: number
  className: string
}

function RecallDonutChart({ categories }: { categories: RecallChartCategory[] }) {
  const total = categories.reduce((sum, category) => sum + category.value, 0)
  let offset = 0

  return (
    <div className="recall-donut" role="img" aria-label={`Today's recall state: ${total} questions`}>
      <svg viewBox="0 0 120 120" role="img" aria-hidden="true">
        <circle className="recall-donut-track" cx="60" cy="60" r="46" pathLength="100" />
        {total > 0
          ? categories.map((category) => {
              const percentage = (category.value / total) * 100
              const segmentOffset = offset
              offset += percentage

              if (category.value === 0) {
                return null
              }

              return (
                <circle
                  key={category.label}
                  className={`recall-donut-segment ${category.className}`}
                  cx="60"
                  cy="60"
                  r="46"
                  pathLength="100"
                  strokeDasharray={`${percentage} ${100 - percentage}`}
                  strokeDashoffset={-segmentOffset}
                />
              )
            })
          : null}
      </svg>
      <div className="recall-donut-center" aria-hidden="true">
        <strong>{total}</strong>
        <span>Today</span>
      </div>
    </div>
  )
}

async function reviewQuestion(questionId: number, rating: SchedulerRating): Promise<void> {
  await invoke('review_question', { questionId, rating, isCorrect: rating === 'easy' })
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
  const location = useLocation()
  const requestedSpaceId =
    typeof location.state?.recallSpaceId === 'number' ? location.state.recallSpaceId : null
  const requestedSpaceName =
    typeof location.state?.recallSpaceName === 'string' ? location.state.recallSpaceName : null
  const didStartRequestedSpace = useRef(false)
  const [dashboard, setDashboard] = useState<RecallDashboard | null>(null)
  const [isDashboardLoading, setIsDashboardLoading] = useState(true)
  const [isSessionLoading, setIsSessionLoading] = useState(false)
  const [isInSession, setIsInSession] = useState(false)
  const [dueQuestions, setDueQuestions] = useState<SessionQuestion[]>([])
  const [currentIndex, setCurrentIndex] = useState(0)
  const [selectedOptionId, setSelectedOptionId] = useState<OptionId | null>(null)
  const [isSubmitted, setIsSubmitted] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [reviews, setReviews] = useState<SessionReview[]>([])
  const [sessionTitle, setSessionTitle] = useState("Today's Recall")
  const [selectedSpaceId, setSelectedSpaceId] = useState<number | null>(requestedSpaceId)

  const totalQuestions = dueQuestions.length
  const isComplete = currentIndex >= totalQuestions
  const currentQuestion = isComplete ? null : dueQuestions[currentIndex]

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

  const selectedSpace = useMemo(
    () => dashboard?.spaces.find((space) => space.id === selectedSpaceId) ?? null,
    [dashboard, selectedSpaceId],
  )

  const scopeMetrics = useMemo(() => {
    const dueToday = selectedSpace
      ? Math.max(0, selectedSpace.due_count - selectedSpace.overdue_count)
      : dashboard?.due_today_count ?? 0
    const overdue = selectedSpace?.overdue_count ?? dashboard?.overdue_count ?? 0
    const reviewed = selectedSpace?.reviewed_today_count ?? dashboard?.reviewed_today_count ?? 0
    const reviewedCorrect = selectedSpace?.correct_today_count ?? dashboard?.correct_today_count ?? 0

    return {
      dueToday,
      overdue,
      reviewedCorrect,
      reviewedIncorrect: Math.max(0, reviewed - reviewedCorrect),
      attention: dueToday + overdue,
      totalQuestions:
        selectedSpace?.total_questions ??
        dashboard?.spaces.reduce((sum, space) => sum + space.total_questions, 0) ??
        0,
    }
  }, [dashboard, selectedSpace])

  const chartCategories = useMemo<RecallChartCategory[]>(
    () => [
      { label: 'Due Today', value: scopeMetrics.dueToday, className: 'is-due' },
      { label: 'Overdue', value: scopeMetrics.overdue, className: 'is-overdue' },
      { label: 'Reviewed Correct', value: scopeMetrics.reviewedCorrect, className: 'is-correct' },
      { label: 'Reviewed Incorrect', value: scopeMetrics.reviewedIncorrect, className: 'is-incorrect' },
    ],
    [scopeMetrics],
  )

  const applySessionQuestions = useCallback((questions: StoredQuestion[]) => {
    setDueQuestions(questions.map(toSessionQuestion))
    setCurrentIndex(0)
    setSelectedOptionId(null)
    setIsSubmitted(false)
    setIsSubmitting(false)
    setReviews([])
  }, [])

  const loadDashboard = useCallback(async () => {
    setIsDashboardLoading(true)

    try {
      const summary = await invoke<RecallDashboard>('get_recall_dashboard')
      setDashboard(summary)
      setError('')
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load recall dashboard'
      setError(message)
    } finally {
      setIsDashboardLoading(false)
    }
  }, [])

  useEffect(() => {
    let isCancelled = false

    void invoke<RecallDashboard>('get_recall_dashboard')
      .then((summary) => {
        if (isCancelled) return
        setDashboard(summary)
        setError('')
      })
      .catch((err: unknown) => {
        if (isCancelled) return
        setError(err instanceof Error ? err.message : 'Failed to load recall dashboard')
      })
      .finally(() => {
        if (!isCancelled) {
          setIsDashboardLoading(false)
        }
      })

    return () => {
      isCancelled = true
    }
  }, [])

  const startRecall = useCallback(async (spaceId: number | null = null, title = "Today's Recall") => {
    setIsSessionLoading(true)
    setError('')
    setSessionTitle(title)
    setSelectedSpaceId(spaceId)

    try {
      const questions = await invoke<StoredQuestion[]>('get_due_questions', {
        spaceId,
      })

      if (questions.length === 0) {
        await loadDashboard()
        return
      }

      applySessionQuestions(questions)
      setIsInSession(true)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load questions for recall'
      setError(message)
    } finally {
      setIsSessionLoading(false)
    }
  }, [applySessionQuestions, loadDashboard])

  useEffect(() => {
    if (requestedSpaceId === null || didStartRequestedSpace.current) {
      return
    }

    didStartRequestedSpace.current = true
    const launchTimer = window.setTimeout(() => {
      void startRecall(requestedSpaceId, requestedSpaceName ?? 'Recall Space')
    }, 0)

    return () => {
      window.clearTimeout(launchTimer)
    }
  }, [requestedSpaceId, requestedSpaceName, startRecall])

  const handleSubmit = async () => {
    if (!currentQuestion || !selectedOptionId || isSubmitted || isSubmitting) {
      return
    }

    const isCorrect = selectedOptionId === currentQuestion.correctOptionId
    setIsSubmitting(true)
    setError('')

    try {
      await reviewQuestion(currentQuestion.id, getSchedulerRating(isCorrect))

      setReviews((current) => [
        ...current,
        { questionId: currentQuestion.id, selectedOptionId, isCorrect },
      ])
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

    setCurrentIndex(currentIndex + 1)
    setSelectedOptionId(null)
    setIsSubmitted(false)
  }

  const returnToDashboard = () => {
    setIsInSession(false)
    setDueQuestions([])
    setCurrentIndex(0)
    void loadDashboard()
  }

  if (isInSession) {
    return (
      <div className="app-container session-page" aria-label="Recall session">
        {error ? <div className="error-banner">{error}</div> : null}

        {isComplete ? (
          <SessionComplete
            reviewedCount={reviews.length}
            correctCount={correctCount}
            onReturn={returnToDashboard}
          />
        ) : currentQuestion ? (
          <div className="session-shell">
            <button type="button" className="btn-back recall-back-btn" onClick={returnToDashboard}>
              <ArrowLeft className="size-4" aria-hidden="true" />
              Back to Recall
            </button>

            <SessionHeader
              recallSpaceName={sessionTitle}
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
                    <ArrowRight className="size-4" aria-hidden="true" />
                  </button>
                </div>
              </>
            ) : null}
          </div>
        ) : null}
      </div>
    )
  }

  return (
    <div className="app-container recall-page" aria-label="Recall dashboard">
      <header className="recall-page-header">
        <BackToHome />
        <div>
          <h1>Recall</h1>
          <p>Keep your knowledge fresh with today&apos;s scheduled reviews.</p>
        </div>
      </header>

      {error ? <div className="error-banner">{error}</div> : null}

      {isDashboardLoading || !dashboard ? (
        <section className="recall-loading surface-panel" aria-live="polite">
          <RotateCcw className="recall-loading-icon" aria-hidden="true" />
          <p>Loading your recall queue…</p>
        </section>
      ) : (
        <>
          <section className="recall-queue-workspace surface-panel" aria-labelledby="todays-recall-heading">
            <div className="recall-queue-head">
              <div className="recall-queue-intro">
                <span className="recall-queue-symbol" aria-hidden="true">
                  <BookOpenCheck />
                </span>
                <div>
                  <p className="recall-section-label">Today&apos;s recall</p>
                  <h2 id="todays-recall-heading">
                    {scopeMetrics.attention > 0 ? 'Your review queue is ready.' : 'You’re all caught up.'}
                  </h2>
                  <p>
                    {selectedSpace
                      ? `${selectedSpace.name} has ${questionLabel(scopeMetrics.attention)} ready for review.`
                      : `${questionLabel(scopeMetrics.attention)} are waiting across all recall spaces.`}
                  </p>
                </div>
              </div>

              <label className="recall-scope-control recall-space-select-wrap">
                <span>Session scope</span>
                <select
                  className="recall-space-select"
                  value={selectedSpaceId === null ? 'all' : String(selectedSpaceId)}
                  onChange={(event) => {
                    setSelectedSpaceId(event.target.value === 'all' ? null : Number(event.target.value))
                  }}
                  disabled={isSessionLoading}
                >
                  <option value="all">All Recall Spaces</option>
                  {dashboard.spaces.map((space) => (
                    <option key={space.id} value={space.id}>
                      {space.name}
                    </option>
                  ))}
                </select>
                <ChevronDown className="recall-space-chevron" aria-hidden="true" />
              </label>
            </div>

            <div className="recall-state-layout">
              <RecallDonutChart categories={chartCategories} />

              <dl className="recall-chart-legend" aria-label="Today's recall state counts">
                {chartCategories.map((category) => (
                  <div key={category.label}>
                    <span className={`recall-chart-swatch ${category.className}`} aria-hidden="true" />
                    <dt>{category.label}</dt>
                    <dd>{category.value}</dd>
                  </div>
                ))}
              </dl>
            </div>

            <div className="recall-queue-commit">
              <p>
                <strong>
                  {selectedSpace ? selectedSpace.name : 'All Recall Spaces'} · {questionLabel(scopeMetrics.totalQuestions)} total
                </strong>
                <span>{questionLabel(scopeMetrics.attention)} currently due for recall.</span>
              </p>
              <button
                type="button"
                className="btn-primary recall-start-btn"
                onClick={() => {
                  void startRecall(selectedSpaceId, selectedSpace?.name ?? "Today's Recall")
                }}
                disabled={scopeMetrics.attention === 0 || isSessionLoading}
              >
                {isSessionLoading ? 'Preparing recall…' : `Start Recall ${scopeMetrics.attention}`}
                <ArrowRight className="size-4" aria-hidden="true" />
              </button>
            </div>
          </section>

          <section className="recall-space-overview" aria-labelledby="recall-space-overview-heading">
            <div className="recall-space-overview-head">
              <div>
                <h2 id="recall-space-overview-heading">Recall by Space</h2>
                <p>Start a focused session without changing the default global queue.</p>
              </div>
            </div>

            <div className="recall-space-overview-list">
              {dashboard.spaces.map((space) => {
                const dueToday = Math.max(0, space.due_count - space.overdue_count)

                return (
                  <article className="recall-space-overview-row" key={space.id}>
                    <div>
                      <h3>{space.name}</h3>
                      <p>{questionLabel(space.total_questions)} total</p>
                    </div>
                    <dl>
                      <div>
                        <dt>Due</dt>
                        <dd>{dueToday}</dd>
                      </div>
                      <div>
                        <dt>Overdue</dt>
                        <dd>{space.overdue_count}</dd>
                      </div>
                    </dl>
                    <button
                      type="button"
                      className="btn-secondary recall-space-overview-action"
                      onClick={() => {
                        void startRecall(space.id, space.name)
                      }}
                      disabled={space.due_count === 0 || isSessionLoading}
                    >
                      Recall {space.due_count}
                      <ArrowRight className="size-4" aria-hidden="true" />
                    </button>
                  </article>
                )
              })}
            </div>
          </section>
        </>
      )}
    </div>
  )
}
