import { invoke } from '@tauri-apps/api/core'
import { useEffect, useRef, useState } from 'react'
import { studyPreferencesError, type StudyPreferences } from '../learning-items/study-plan'

export function StudyPreferencesSettings() {
  const [preferences, setPreferences] = useState<StudyPreferences | null>(null)
  const [target, setTarget] = useState('')
  const [maximumNew, setMaximumNew] = useState('')
  const [applyToday, setApplyToday] = useState(false)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [status, setStatus] = useState('')
  const heading = useRef<HTMLHeadingElement>(null)
  const validation = studyPreferencesError(target, maximumNew)

  const load = async () => {
    setLoading(true)
    setError('')
    try {
      const value = await invoke<StudyPreferences>('get_study_preferences')
      setPreferences(value)
      setTarget(String(value.daily_target))
      setMaximumNew(String(value.max_new_items))
    } catch (err) {
      setError(`Unable to load study preferences: ${String(err)}`)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    let cancelled = false
    void invoke<StudyPreferences>('get_study_preferences').then((value) => {
      if (cancelled) return
      setPreferences(value)
      setTarget(String(value.daily_target))
      setMaximumNew(String(value.max_new_items))
    }).catch((err: unknown) => {
      if (!cancelled) setError(`Unable to load study preferences: ${String(err)}`)
    }).finally(() => {
      if (!cancelled) setLoading(false)
    })
    if (window.location.hash === '#study-preferences') heading.current?.focus()
    return () => { cancelled = true }
  }, [])

  const save = async (event: React.FormEvent) => {
    event.preventDefault()
    if (validation || saving) return
    setSaving(true)
    setError('')
    setStatus('')
    try {
      const value = await invoke<StudyPreferences>('save_study_preferences', {
        preferences: { daily_target: Number(target), max_new_items: Number(maximumNew) }, applyToday,
      })
      setPreferences(value)
      setStatus(applyToday ? 'Saved. Today’s remaining plan has been updated; completed work is kept.' : 'Saved. Your next daily plan will use these limits.')
      setApplyToday(false)
    } catch (err) {
      setError(`Unable to save study preferences: ${String(err)}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <section id="study-preferences" className="settings-panel" aria-labelledby="study-preferences-heading">
      <header className="settings-section-head">
        <h2 id="study-preferences-heading" ref={heading} tabIndex={-1}>Study preferences</h2>
        <p>Choose a daily workload shared across all your Recall Spaces. Due reviews take priority.</p>
      </header>
      {loading ? <p role="status">Loading study preferences…</p> : preferences ? (
        <form className="settings-stack" onSubmit={(event) => void save(event)}>
          <div className="study-preferences-fields">
            <label className="settings-field" htmlFor="daily-study-target">
              <span id="daily-study-target-label">Daily study target</span>
              <input id="daily-study-target" className="settings-input" type="number" min="1" max="10000" step="1" required
                value={target} disabled={saving} onChange={(event) => { setTarget(event.target.value); setStatus('') }}
                aria-labelledby="daily-study-target-label" aria-describedby="daily-study-target-help study-preferences-validation" />
              <span id="daily-study-target-help" className="settings-help-text">Up to this many items per day, including reviews and new items.</span>
            </label>
            <label className="settings-field" htmlFor="maximum-new-items">
              <span id="maximum-new-items-label">Maximum new items per day</span>
              <input id="maximum-new-items" className="settings-input" type="number" min="0" max={Number(target) || 0} step="1" required
                value={maximumNew} disabled={saving} onChange={(event) => { setMaximumNew(event.target.value); setStatus('') }}
                aria-labelledby="maximum-new-items-label" aria-describedby="maximum-new-items-help study-preferences-validation" />
              <span id="maximum-new-items-help" className="settings-help-text">Included in your daily target. Choose 0 to study scheduled reviews only.</span>
            </label>
          </div>
          <p id="study-preferences-validation" className="settings-status is-error" aria-live="polite">{validation}</p>
          <label className="study-preferences-apply">
            <input type="checkbox" checked={applyToday} disabled={saving} onChange={(event) => setApplyToday(event.target.checked)} />
            Update today’s remaining plan too
          </label>
          <p className="settings-help-text">Otherwise, changes apply to your next daily plan. Items already studied are always kept in your progress.</p>
          <div className="settings-actions">
            <button className="btn-primary" type="submit" disabled={saving || Boolean(validation)}>{saving ? 'Saving…' : 'Save study preferences'}</button>
            <p role="status" className="settings-status">{status}</p>
          </div>
        </form>
      ) : <button type="button" className="btn-secondary" onClick={() => void load()}>Retry loading preferences</button>}
      {error ? <p role="alert" className="settings-status is-error">{error}</p> : null}
    </section>
  )
}
