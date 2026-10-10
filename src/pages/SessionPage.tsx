import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowRight } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { BackToHome } from '../components/BackToHome'
import { RecallDonutChart } from '../components/RecallDonutChart'
import { RecallCard } from '../components/session/RecallCard'
import { SessionHeader } from '../components/session/SessionHeader'
import { selectRecallVariant, type RecallItem } from '../learning-items/recall'
import { dailyPlanStatus, type DailyStudyPlan, type SelfDirectedStudySession, type StudyItemKind } from '../learning-items/study-plan'
import type { LearningItem } from '../learning-items/types'

type Space = { id: number; name: string }
type SessionItem = { item: RecallItem; isExtra: boolean }
type RecallDashboard = {
  spaces: { total_questions: number }[]
}

// Load a bounded batch at a time; this is not a limit on the study session.
const STUDY_BATCH_SIZE = 20

function sessionItems(items: LearningItem[]): SessionItem[] {
  return items.map((raw) => {
    const item = selectRecallVariant(raw)
    if (!item) throw new Error('An available item has no supported question. Open its Space to repair it.')
    return { item, isExtra: false }
  })
}

export function SessionPage() {
  const location = useLocation()
  // Navigation starts a fresh view, so pending work cannot reopen the old session.
  return <StudySession key={location.key} />
}

function StudySession() {
  const location = useLocation()
  const requestedSpaceId = typeof location.state?.recallSpaceId === 'number' ? location.state.recallSpaceId as number : null
  const [plan, setPlan] = useState<DailyStudyPlan | null>(null)
  const [spaces, setSpaces] = useState<Space[]>([])
  const [dashboard, setDashboard] = useState<RecallDashboard | null>(null)
  const [selectedSpaceId, setSelectedSpaceId] = useState<number | null>(requestedSpaceId)
  const [studyKind, setStudyKind] = useState<StudyItemKind>('all')
  const [sessionSelfDirected, setSessionSelfDirected] = useState(false)
  const [sessionKind, setSessionKind] = useState<StudyItemKind>('all')
  const [sessionStudiedCount, setSessionStudiedCount] = useState(0)
  const [awaitingItems, setAwaitingItems] = useState(false)
  const [scopePreview, setScopePreview] = useState<{
    source: DailyStudyPlan
    spaceId: number | null
    kind: StudyItemKind
    session: SelfDirectedStudySession | null
    error: string
  } | null>(null)
  const [sessionSpaceId, setSessionSpaceId] = useState<number | null>(null)
  const [sessionDate, setSessionDate] = useState('')
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
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
      setNotice('')
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

  useEffect(() => {
    if (!plan || current) return
    let cancelled = false
    void invoke<SelfDirectedStudySession>('get_self_directed_study_session', {
      spaceId: selectedSpaceId, kind: studyKind, limit: 1,
    }).then((next) => {
      if (!cancelled) setScopePreview({ source: plan, spaceId: selectedSpaceId, kind: studyKind, session: next, error: '' })
    }).catch((err: unknown) => {
      if (!cancelled) setScopePreview({ source: plan, spaceId: selectedSpaceId, kind: studyKind, session: null,
        error: 'Unable to check available study items: ' + String(err) })
    })
    return () => { cancelled = true }
  }, [plan, selectedSpaceId, studyKind, current])

  const start = async (selfDirected = false) => {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      let rawItems: LearningItem[]
      let date: string
      const spaceId = selfDirected ? selectedSpaceId : null
      if (selfDirected) {
        const next = await invoke<SelfDirectedStudySession>('get_self_directed_study_session', {
          spaceId, kind: studyKind, limit: STUDY_BATCH_SIZE,
        })
        rawItems = next.items
        date = next.local_date
      } else {
        const next = await invoke<DailyStudyPlan>('get_daily_study_plan', { spaceId: null })
        rawItems = next.items.map((assignment) => assignment.item)
        date = next.local_date
      }

      const items = sessionItems(rawItems)

      if (items.length === 0) {
        await loadDashboard()
        setNotice('No new or currently due items match this selection. Future reviews remain scheduled.')
        return
      }

      setSessionSpaceId(spaceId)
      setSessionDate(date)
      setSessionSelfDirected(selfDirected)
      setSessionKind(studyKind)
      setSessionStudiedCount(0)
      setAwaitingItems(false)
      setQueue(items)
      setIndex(0)
    } catch (err) { setError(String(err)) }
    finally { setBusy(false) }
  }

  const returnToPlan = async () => {
    setQueue([])
    setAwaitingItems(false)
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

  const continueSelfDirected = async () => {
    setAwaitingItems(true)
    setBusy(true)
    setError('')
    try {
      const next = await invoke<SelfDirectedStudySession>('get_self_directed_study_session', {
        spaceId: sessionSpaceId, kind: sessionKind, limit: STUDY_BATCH_SIZE,
      })
      if (next.local_date !== sessionDate) {
        await returnToPlan()
        setNotice('A new study day has started. Choose what to study next.')
        return
      }
      const items = sessionItems(next.items)
      if (items.length === 0) {
        await returnToPlan()
        setNotice('No unstudied new or due items remain in this selection. Future reviews stay scheduled.')
        return
      }
      setQueue(items)
      setIndex(0)
      setAwaitingItems(false)
    } catch (err) { setError('Unable to load the next item: ' + String(err)) }
    finally { setBusy(false) }
  }

  const advance = () => {
    if (index < queue.length - 1) setIndex((value) => value + 1)
    else if (sessionSelfDirected) void continueSelfDirected()
    else void returnToPlan()
  }

  const scopeName = spaces.find((space) => space.id === sessionSpaceId)?.name ?? 'All Recall Spaces'
  if (current) {
    return (
      <div className="app-container session-page" aria-label="Study session">
        <div className="session-shell">
          <button className="btn-back recall-back-btn" type="button" disabled={submitting || busy}
            onClick={() => void returnToPlan()}><ArrowLeft className="size-4" aria-hidden="true" />{sessionSelfDirected ? 'End session' : 'Back to today’s plan'}</button>
          <SessionHeader sessionLabel={sessionSelfDirected ? 'Self-directed study' : 'Suggested plan'} recallSpaceName={scopeName}
            currentItemNumber={index + 1} totalItems={queue.length}
            studiedCount={sessionSelfDirected ? sessionStudiedCount : undefined} />
          {notice ? <p className="session-review-status" role="status">{notice}</p> : null}
          {awaitingItems ? (
            <div className="surface-panel" aria-busy={busy}>
              {busy ? <p role="status">Loading the next item…</p> : <>
                <p className="error-banner" role="alert">{error}</p>
                <button className="btn-primary" type="button" onClick={() => void continueSelfDirected()}>Retry loading next item</button>
              </>}
            </div>
          ) : <RecallCard key={current.item.learningItemId} item={current.item} planDate={sessionDate}
            spaceId={sessionSpaceId} isExtra={current.isExtra} selfDirected={sessionSelfDirected}
            isLast={!sessionSelfDirected && index === queue.length - 1} onBusy={setSubmitting}
            onReviewed={() => setSessionStudiedCount((value) => value + 1)}
            onNext={() => {
              setNotice('')
              advance()
            }}
            onDeleted={() => {
              setNotice('Question deleted.')
              if (index === queue.length - 1) advance()
              else setQueue((items) => items.filter((entry) => entry.item.learningItemId !== current.item.learningItemId))
            }} />}
        </div>
      </div>
    )
  }

  const remaining = plan?.items.filter((item) => !item.is_extra) ?? []
  const reviews = remaining.filter((item) => !item.was_new).length
  const newItems = remaining.filter((item) => item.was_new).length
  const status = plan ? dailyPlanStatus(plan) : null
  const hasNoStudyItems = Boolean(plan && plan.completed_count === 0 && dashboard &&
    dashboard.spaces.every((space) => space.total_questions === 0))
  const unfilledTarget = plan
    ? Math.max(0, plan.daily_target - plan.completed_count - remaining.length)
    : 0
  const scopeMatches = scopePreview?.source === plan && scopePreview?.spaceId === selectedSpaceId && scopePreview?.kind === studyKind
  const available = scopeMatches ? scopePreview?.session ?? null : null
  const scopeError = scopeMatches ? scopePreview?.error ?? '' : ''
  const checkingAvailability = Boolean(plan && !available && !scopeError)

  return (
    <div className="app-container recall-page" aria-label="Recall dashboard">
      <header className="recall-page-header">
        <BackToHome />
        <div><h1>Recall</h1><p>A manageable plan for today, at your pace.</p></div>
      </header>
      {error ? <div className="error-banner" role="alert">{error}</div> : null}
      {notice ? <p className="session-review-status" role="status">{notice}</p> : null}
      {loading ? <p role="status">Loading today’s plan…</p> : !plan ? (
        <button className="btn-secondary" type="button" disabled={busy} onClick={() => void loadDashboard()}>Retry loading plan</button>
      ) : (
        <>
          <section className="daily-plan surface-panel" aria-labelledby="daily-plan-heading" aria-busy={busy}>
            <div className="daily-plan-heading">
              <h2 id="daily-plan-heading" ref={planHeading} tabIndex={-1}>
                {hasNoStudyItems ? 'Nothing to study yet'
                  : status === 'goal-reached' ? 'Daily goal reached'
                  : status === 'new-limit-reached' ? 'Daily new-item limit reached'
                  : status === 'no-eligible-reviews' ? 'No eligible reviews remain'
                  : 'Suggested plan'}
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
                ) : (
                  <>
                    <p className="daily-plan-progress">{plan.completed_count} of {plan.daily_target} {plan.daily_target === 1 ? 'item' : 'items'} studied across all Recall Spaces</p>
                    {status === 'ready' ? (
                      <p className="daily-plan-detail">{remaining.length} remaining · {reviews} {reviews === 1 ? 'review' : 'reviews'} and {newItems} new</p>
                    ) : status === 'new-limit-reached' ? (
                      <p className="daily-plan-detail">No eligible reviews remain. {plan.max_new_items === 0
                        ? 'Your suggested plan allows no new items.'
                        : 'You have used the suggested plan’s new-item allowance.'} You can still choose new items in self-directed study below.</p>
                    ) : status === 'no-eligible-reviews' ? (
                      <p className="daily-plan-detail">No items are eligible for today’s plan right now. Your daily goal has not been reached.</p>
                    ) : null}
                  </>
                )}
                {!hasNoStudyItems ? (
                  <p className="daily-plan-detail">
                    Daily goal: {plan.daily_target} total items. The suggested plan includes up to {plan.max_new_items} new items. Study any new or due items below to work toward the same goal.
                    {plan.total_count < plan.daily_target && status === 'ready' ? ' Today only includes currently eligible items.' : ''}
                  </p>
                ) : null}
                {plan.extra_completed_count > 0 ? <p className="daily-plan-detail">Includes {plan.extra_completed_count} {plan.extra_completed_count === 1 ? 'item' : 'items'} from extra study.</p> : null}
                {remaining.length > 0 ? <div className="daily-plan-actions">
                  <button className="btn-secondary" type="button" disabled={busy} onClick={() => void start()}>
                    Follow suggested plan<ArrowRight className="size-4" aria-hidden="true" />
                  </button>
                </div> : null}
              </div>
              <RecallDonutChart label="Today's plan breakdown across all Recall Spaces"
                centerValue={`${plan.completed_count} / ${plan.daily_target}`}
                centerLabel="Studied today"
                categories={[
                  { label: 'Completed', value: plan.completed_count, tone: 'correct' },
                  { label: 'Reviews remaining', value: reviews, tone: 'due' },
                  { label: 'New remaining', value: newItems, tone: 'new' },
                  ...(unfilledTarget > 0
                    ? [{ label: 'No eligible items', value: unfilledTarget, tone: 'muted' as const }]
                    : []),
                ]} />
            </div>
            <section className="daily-plan-scope" aria-labelledby="study-scope-heading">
              <div>
                <h3 id="study-scope-heading">Study your way</h3>
                <p className="daily-plan-detail">Choose what to study and stop whenever you want. Your work counts toward today’s goal, even beyond the suggested limits.</p>
              </div>
              <div className="self-directed-controls">
              <label className="settings-field" htmlFor="study-space">
                <span id="study-space-label">Recall Space</span>
                <select id="study-space" className="settings-input" value={selectedSpaceId ?? ''} disabled={busy}
                  aria-labelledby="study-space-label"
                  onChange={(event) => setSelectedSpaceId(event.target.value ? Number(event.target.value) : null)}>
                  <option value="">All Recall Spaces</option>
                  {spaces.map((space) => <option key={space.id} value={space.id}>{space.name}</option>)}
                </select>
              </label>
              <label className="settings-field" htmlFor="study-kind">
                <span>Items to study</span>
                <select id="study-kind" className="settings-input" value={studyKind} disabled={busy}
                  onChange={(event) => setStudyKind(event.target.value as StudyItemKind)}>
                  <option value="all">New and due items</option>
                  <option value="new">New items</option>
                  <option value="reviews">Due reviews</option>
                </select>
              </label>
              </div>
              <p className="daily-plan-detail">Continue through new and due items at your own pace. Future reviews stay scheduled for their due dates.</p>
              <div aria-live="polite" aria-busy={checkingAvailability}>
                {checkingAvailability ? <p className="daily-plan-detail">Checking available items…</p>
                  : scopeError ? <p className="daily-plan-detail" role="alert">{scopeError}</p>
                  : available?.items.length === 0 && !hasNoStudyItems ? (
                    <p className="daily-plan-detail">{studyKind === 'new' ? 'No unstudied new items match this selection.'
                      : studyKind === 'reviews' ? 'No unstudied reviews are due in this selection.'
                      : 'No unstudied new or due items match this selection.'} Try another Space or item type. Future reviews remain scheduled.</p>
                  ) : null}
              </div>
            <div className="daily-plan-actions">
              {hasNoStudyItems ? (
                <Link className="btn-primary" to="/">Add learning items<ArrowRight className="size-4" aria-hidden="true" /></Link>
              ) : (
                <>
                  <button className="btn-primary" type="button"
                    disabled={busy || checkingAvailability || !available?.items.length}
                    onClick={() => void start(true)}>
                    {busy ? 'Loading…' : selectedSpaceId === null ? 'Study across all Spaces' : 'Study this Space'}
                    <ArrowRight className="size-4" aria-hidden="true" />
                  </button>
                  {status === 'goal-reached' ? <Link to="/">Done for today</Link> : null}
                  {scopeError ? <button className="btn-secondary" type="button" disabled={busy}
                    onClick={() => void loadDashboard().catch((err: unknown) => setError(String(err)))}>Retry availability</button> : null}
                </>
              )}
              <Link to="/questions">View all Spaces and workload<ArrowRight className="size-4" aria-hidden="true" /></Link>
            </div>
          </section>
          </section>
        </>
      )}
    </div>
  )
}
