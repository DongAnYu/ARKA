import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowRight } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { BackToHome } from '../components/BackToHome'
import { RecallDonutChart } from '../components/RecallDonutChart'
import { RecallCard } from '../components/session/RecallCard'
import { SessionHeader } from '../components/session/SessionHeader'
import { selectRecallVariant, type RecallItem } from '../learning-items/recall'
import type { DailyStudyPlan } from '../learning-items/study-plan'

type Space = { id: number; name: string }
type SessionItem = { item: RecallItem; isExtra: boolean }
type RecallSpaceSummary = {
  id: number
  name: string
  total_questions: number
  due_count: number
  overdue_count: number
  new_count: number
  reviewed_today_count: number
}
type RecallDashboard = {
  due_today_count: number
  overdue_count: number
  new_count: number
  reviewed_today_count: number
  spaces: RecallSpaceSummary[]
}

export function SessionPage() {
  const location = useLocation()
  const requestedSpaceId = typeof location.state?.recallSpaceId === 'number' ? location.state.recallSpaceId as number : null
  const [plan, setPlan] = useState<DailyStudyPlan | null>(null)
  const [spaces, setSpaces] = useState<Space[]>([])
  const [dashboard, setDashboard] = useState<RecallDashboard | null>(null)
  const [selectedSpaceId, setSelectedSpaceId] = useState<number | null>(requestedSpaceId)
  const [sessionSpaceId, setSessionSpaceId] = useState<number | null>(null)
  const [sessionDate, setSessionDate] = useState('')
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [queue, setQueue] = useState<SessionItem[]>([])
  const [index, setIndex] = useState(0)
  const planHeading = useRef<HTMLHeadingElement>(null)
  const current = queue[index]

  const loadDashboard = useCallback(async () => {
    const [next, availableSpaces, nextDashboard] = await Promise.all([
      invoke<DailyStudyPlan>('get_daily_study_plan', { spaceId: null }),
      invoke<Space[]>('get_spaces'),
      invoke<RecallDashboard>('get_recall_dashboard'),
    ])
    setPlan(next)
    setSpaces(availableSpaces)
    setDashboard(nextDashboard)
    return { plan: next, spaces: availableSpaces }
  }, [])

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      setLoading(true)
      setError('')
      try {
        const result = await loadDashboard()
        if (!cancelled) {
          const requestedExists = requestedSpaceId !== null && result.spaces.some((space) => space.id === requestedSpaceId)
          setSelectedSpaceId(requestedExists ? requestedSpaceId : null)
          setSessionSpaceId(null)
          setSessionDate('')
          setQueue([])
        }
      } catch (err) {
        if (!cancelled) setError('Unable to load today’s plan: ' + String(err))
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void load()
    return () => { cancelled = true }
  }, [loadDashboard, requestedSpaceId, location.key])

  useEffect(() => {
    const onFocus = () => {
      if (!current && !busy) void loadDashboard().catch((err: unknown) => setError(String(err)))
    }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [current, busy, loadDashboard])

  const start = async (extra = false) => {
    setBusy(true)
    setError('')
    try {
      let next: DailyStudyPlan
      if (extra) {
        next = await invoke<DailyStudyPlan>('extend_daily_study_plan', { spaceId: selectedSpaceId })
      } else {
        next = await invoke<DailyStudyPlan>('get_daily_study_plan', { spaceId: selectedSpaceId })
      }

      const items: SessionItem[] = []
      for (const assignment of next.items) {
        const item = selectRecallVariant(assignment.item)
        if (!item) throw new Error('An assigned item has no supported question. Open its Space to repair it.')
        items.push({ item, isExtra: assignment.is_extra })
      }

      if (items.length === 0) {
        await loadDashboard()
        const selectedName = spaces.find((space) => space.id === selectedSpaceId)?.name
        setError(selectedName ? `No eligible items to study in ${selectedName} right now.` : 'No eligible items to study right now.')
        return
      }

      setSessionSpaceId(selectedSpaceId)
      setSessionDate(next.local_date)
      setQueue(items)
      setIndex(0)
    } catch (err) { setError(String(err)) }
    finally { setBusy(false) }
  }

  const returnToPlan = async () => {
    setQueue([])
    setBusy(true)
    try {
      await loadDashboard()
      setSessionSpaceId(null)
      setSessionDate('')
      setError('')
      requestAnimationFrame(() => planHeading.current?.focus())
    }
    catch (err) { setError(String(err)) }
    finally { setBusy(false) }
  }

  const scopeName = spaces.find((space) => space.id === sessionSpaceId)?.name ?? 'All Recall Spaces'
  if (current) {
    return (
      <div className="app-container session-page" aria-label="Study session">
        <div className="session-shell">
          <button className="btn-back recall-back-btn" type="button" disabled={submitting}
            onClick={() => void returnToPlan()}><ArrowLeft className="size-4" aria-hidden="true" />Back to today’s plan</button>
          <SessionHeader sessionLabel={current.isExtra ? 'Extra study' : 'Today’s plan'} recallSpaceName={scopeName}
            currentItemNumber={index + 1} totalItems={queue.length} />
          <RecallCard key={current.item.learningItemId} item={current.item} planDate={sessionDate}
            spaceId={sessionSpaceId} isExtra={current.isExtra}
            isLast={index === queue.length - 1} onBusy={setSubmitting} onReviewed={() => { /* Progress is persisted with the review. */ }}
            onNext={() => {
              if (index === queue.length - 1) void returnToPlan()
              else setIndex((value) => value + 1)
            }} />
        </div>
      </div>
    )
  }

  const remaining = plan?.items.filter((item) => !item.is_extra) ?? []
  const reviews = remaining.filter((item) => !item.was_new).length
  const newItems = remaining.filter((item) => item.was_new).length
  const hasNoStudyItems = Boolean(plan && plan.total_count === 0)
  const complete = Boolean(plan && plan.total_count > 0 && remaining.length === 0)
  const unfilledTarget = plan
    ? Math.max(0, plan.daily_target - plan.completed_count - remaining.length)
    : 0
  const extraPending = plan?.items.filter((item) => item.is_extra).length ?? 0
  const selectedSummary = selectedSpaceId === null
    ? null
    : dashboard?.spaces.find((space) => space.id === selectedSpaceId) ?? null
  const insightName = selectedSummary?.name ?? 'All Recall Spaces'
  const insightDueToday = selectedSummary
    ? Math.max(0, selectedSummary.due_count - selectedSummary.overdue_count)
    : dashboard?.due_today_count ?? 0
  const insightOverdue = selectedSummary?.overdue_count ?? dashboard?.overdue_count ?? 0
  const insightNew = selectedSummary?.new_count ?? dashboard?.new_count ?? 0
  const insightReviewed = selectedSummary?.reviewed_today_count ?? dashboard?.reviewed_today_count ?? 0
  const insightTotal = selectedSummary?.total_questions
    ?? dashboard?.spaces.reduce((sum, space) => sum + space.total_questions, 0)
    ?? 0

  return (
    <div className="app-container recall-page" aria-label="Recall dashboard">
      <header className="recall-page-header">
        <BackToHome />
        <div><h1>Recall</h1><p>A manageable plan for today, at your pace.</p></div>
      </header>
      {error ? <div className="error-banner" role="alert">{error}</div> : null}
      {loading ? <p role="status">Loading today’s plan…</p> : !plan ? (
        <button className="btn-secondary" type="button" disabled={busy} onClick={() => void loadDashboard()}>Retry loading plan</button>
      ) : (
        <>
          <section className="daily-plan surface-panel" aria-labelledby="daily-plan-heading" aria-busy={busy}>
            <div className="daily-plan-heading">
              <h2 id="daily-plan-heading" ref={planHeading} tabIndex={-1}>
                {hasNoStudyItems ? 'Nothing to study yet' : complete ? 'Today’s plan complete' : 'Today’s plan'}
              </h2>
              <Link to="/settings#study-preferences">Adjust daily target</Link>
            </div>
            <div className="daily-plan-content">
              <div className="daily-plan-summary">
                {hasNoStudyItems ? (
                  <>
                    <p className="daily-plan-progress">No learning items are ready for recall.</p>
                    <p className="daily-plan-detail">Generate learning items from your notes to build your first study plan.</p>
                  </>
                ) : complete ? (
                  <p className="daily-plan-progress">You studied {plan.completed_count} {plan.completed_count === 1 ? 'item' : 'items'}.</p>
                ) : plan.total_count > 0 ? (
                  <>
                    <p className="daily-plan-progress">{plan.completed_count} of {plan.daily_target} completed</p>
                    <p className="daily-plan-detail">{remaining.length} remaining · {reviews} {reviews === 1 ? 'review' : 'reviews'} and {newItems} new</p>
                  </>
                ) : <p className="daily-plan-progress">No items to study in this plan.</p>}
                {!hasNoStudyItems ? (
                  <p className="daily-plan-detail">
                    Daily target: {plan.daily_target} items, including up to {plan.max_new_items} new.
                    {plan.total_count < plan.daily_target && !complete ? ' Today only includes currently eligible items.' : ''}
                  </p>
                ) : null}
                {plan.extra_completed_count > 0 ? <p className="daily-plan-detail">{plan.extra_completed_count} additional {plan.extra_completed_count === 1 ? 'item' : 'items'} studied today.</p> : null}
                <div className="daily-plan-actions">
                  {remaining.length > 0 ? (
                    <button className="btn-primary" type="button" disabled={busy} onClick={() => void start()}>
                      {busy ? 'Loading…' : plan.completed_count > 0 ? 'Continue studying' : 'Start studying'}<ArrowRight className="size-4" aria-hidden="true" />
                    </button>
                  ) : hasNoStudyItems ? (
                    <Link className="btn-primary" to="/">
                      Add learning items<ArrowRight className="size-4" aria-hidden="true" />
                    </Link>
                  ) : (
                    <>
                      <Link className="btn-primary" to="/">Done for today</Link>
                      {extraPending > 0 || plan.can_study_more ? (
                        <button className="btn-secondary" type="button" disabled={busy} onClick={() => void start(extraPending === 0)}>
                          {busy ? 'Loading…' : extraPending > 0 ? 'Continue extra study (' + extraPending + ')' : 'Study more'}
                        </button>
                      ) : null}
                    </>
                  )}
                </div>
              </div>
              <RecallDonutChart label="Today's plan breakdown"
                centerValue={`${plan.completed_count} / ${plan.daily_target}`}
                centerLabel="Completed"
                categories={[
                  { label: 'Completed', value: plan.completed_count, tone: 'correct' },
                  { label: 'Reviews remaining', value: reviews, tone: 'due' },
                  { label: 'New remaining', value: newItems, tone: 'new' },
                  ...(unfilledTarget > 0
                    ? [{ label: 'No eligible items', value: unfilledTarget, tone: 'muted' as const }]
                    : []),
                ]} />
            </div>
          </section>
          <section className="daily-plan-scope surface-panel" aria-labelledby="study-scope-heading">
            <div className="daily-plan-scope-head">
              <div>
                <h2 id="study-scope-heading">Choose what to study</h2>
                <p>Focus on one Space or study across all Spaces. Your daily target stays the same.</p>
              </div>
              <label className="settings-field" htmlFor="study-space">
                <span id="study-space-label">Recall Space</span>
                <select id="study-space" className="settings-input" value={selectedSpaceId ?? ''} disabled={busy}
                  aria-labelledby="study-space-label"
                  onChange={(event) => setSelectedSpaceId(event.target.value ? Number(event.target.value) : null)}>
                  <option value="">All Recall Spaces</option>
                  {spaces.map((space) => <option key={space.id} value={space.id}>{space.name}</option>)}
                </select>
              </label>
            </div>
            <div className="daily-plan-space-insight" aria-live="polite">
              <div>
                <h3>{insightName} insight</h3>
                <p>{insightTotal} total · {insightReviewed} reviewed today</p>
              </div>
              <RecallDonutChart label={`${insightName} workload breakdown`}
                centerValue={insightDueToday + insightOverdue + insightNew} centerLabel="Due + new items"
                categories={[
                  { label: 'Due today', value: insightDueToday, tone: 'due' },
                  { label: 'Overdue', value: insightOverdue, tone: 'overdue' },
                  { label: 'New', value: insightNew, tone: 'new' },
                ]} />
            </div>
            <p className="daily-plan-detail">The Space selection filters the session you start; it does not change today’s target.</p>
            <Link to="/questions">View all Spaces and workload<ArrowRight className="size-4" aria-hidden="true" /></Link>
          </section>
        </>
      )}
    </div>
  )
}
