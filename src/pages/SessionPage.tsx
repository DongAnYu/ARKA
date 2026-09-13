import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowRight, BookOpenCheck, ChevronDown, RotateCcw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useLocation } from 'react-router-dom'
import { BackToHome } from '../components/BackToHome'
import { RecallCard } from '../components/session/RecallCard'
import { SessionComplete } from '../components/session/SessionComplete'
import { SessionHeader } from '../components/session/SessionHeader'
import { selectRecallVariant, summarizeRecall, type RecallItem, type RecallResult } from '../learning-items/recall'
import type { LearningItem } from '../learning-items/types'

type RecallDashboard = {
  due_today_count: number
  overdue_count: number
  reviewed_today_count: number
  spaces: RecallSpaceSummary[]
}

type RecallSpaceSummary = {
  id: number
  name: string
  total_questions: number
  due_count: number
  overdue_count: number
  reviewed_today_count: number
}

const itemLabel = (count: number) => `${count} ${count === 1 ? 'learning item' : 'learning items'}`

type RecallChartCategory = {
  label: string
  value: number
  className: string
}

function RecallDonutChart({ categories }: { categories: RecallChartCategory[] }) {
  const total = categories.reduce((sum, category) => sum + category.value, 0)
  let offset = 0

  return (
    <div className="recall-donut" role="img" aria-label={`Today's recall state: ${total} learning items`}>
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
  const [dueItems, setDueItems] = useState<RecallItem[]>([])
  const [currentIndex, setCurrentIndex] = useState(0)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [reviews, setReviews] = useState<RecallResult[]>([])
  const [sessionTitle, setSessionTitle] = useState("Today's Recall")
  const [selectedSpaceId, setSelectedSpaceId] = useState<number | null>(requestedSpaceId)

  const totalItems = dueItems.length
  const isComplete = currentIndex >= totalItems
  const currentItem = isComplete ? null : dueItems[currentIndex]

  const totals = summarizeRecall(reviews)

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

    return {
      dueToday,
      overdue,
      reviewed,
      attention: dueToday + overdue,
      totalItems:
        selectedSpace?.total_questions ??
        dashboard?.spaces.reduce((sum, space) => sum + space.total_questions, 0) ??
        0,
    }
  }, [dashboard, selectedSpace])

  const chartCategories = useMemo<RecallChartCategory[]>(
    () => [
      { label: 'Due Today', value: scopeMetrics.dueToday, className: 'is-due' },
      { label: 'Overdue', value: scopeMetrics.overdue, className: 'is-overdue' },
      { label: 'Reviewed today', value: scopeMetrics.reviewed, className: 'is-correct' },
    ],
    [scopeMetrics],
  )

  const applySessionItems = useCallback((items: RecallItem[]) => {
    setDueItems(items)
    setCurrentIndex(0)
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
      const items = await invoke<LearningItem[]>('get_due_learning_items', {
        spaceId,
      })

      const queue = items.map(selectRecallVariant).filter((item): item is RecallItem => item !== null)
      if (queue.length === 0) {
        await loadDashboard()
        return
      }

      applySessionItems(queue)
      setIsInSession(true)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load learning items for recall'
      setError(message)
    } finally {
      setIsSessionLoading(false)
    }
  }, [applySessionItems, loadDashboard])

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

  const returnToDashboard = () => {
    if (isSubmitting) return
    setIsInSession(false)
    setDueItems([])
    setCurrentIndex(0)
    void loadDashboard()
  }

  if (isInSession) {
    return (
      <div className="app-container session-page" aria-label="Recall session">
        {error ? <div className="error-banner" role="alert">{error}</div> : null}

        {isComplete ? (
          <SessionComplete
            reviewedCount={reviews.length}
            recalledCount={totals.recalled}
            onReturn={returnToDashboard}
          />
        ) : currentItem ? (
          <div className="session-shell">
            <button type="button" className="btn-back recall-back-btn" onClick={returnToDashboard} disabled={isSubmitting}>
              <ArrowLeft className="size-4" aria-hidden="true" />
              Back to Recall
            </button>

            <SessionHeader
              recallSpaceName={sessionTitle}
              currentItemNumber={currentIndex + 1}
              totalItems={totalItems}
            />

            <RecallCard
              key={currentItem.learningItemId}
              item={currentItem}
              isLast={currentIndex === totalItems - 1}
              onReviewed={(result) => setReviews((current) => [...current, result])}
              onBusy={setIsSubmitting}
              onNext={() => setCurrentIndex((index) => index + 1)}
            />
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

      {error ? <div className="error-banner" role="alert">{error}</div> : null}

      {!isDashboardLoading && !dashboard ? (
        <button type="button" className="btn-secondary" onClick={() => { void loadDashboard() }}>Retry loading recall</button>
      ) : isDashboardLoading || !dashboard ? (
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
                      ? `${selectedSpace.name} has ${itemLabel(scopeMetrics.attention)} ready for review.`
                      : `${itemLabel(scopeMetrics.attention)} are waiting across all recall spaces.`}
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
                  {selectedSpace ? selectedSpace.name : 'All Recall Spaces'} · {itemLabel(scopeMetrics.totalItems)} total
                </strong>
                <span>{itemLabel(scopeMetrics.attention)} currently due for recall.</span>
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
                      <p>{itemLabel(space.total_questions)} total</p>
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
