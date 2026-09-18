import { invoke } from '@tauri-apps/api/core'
import { useEffect, useState } from 'react'
import {
  MAX_DISPLAY_NAME_LENGTH,
  userProfileError,
  type UserProfile,
} from '../profile'

export function ProfileSettings() {
  const [profile, setProfile] = useState<UserProfile | null>(null)
  const [displayName, setDisplayName] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [status, setStatus] = useState('')
  const validation = userProfileError(displayName)
  const hasChanges = profile !== null && displayName !== profile.display_name

  const applyProfile = (value: UserProfile) => {
    setProfile(value)
    setDisplayName(value.display_name)
  }

  const load = async () => {
    setLoading(true)
    setError('')
    try {
      applyProfile(await invoke<UserProfile>('get_user_profile'))
    } catch (err) {
      setError(`Unable to load your personalisation settings: ${String(err)}`)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    let cancelled = false

    void invoke<UserProfile>('get_user_profile')
      .then((value) => {
        if (!cancelled) applyProfile(value)
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(`Unable to load your personalisation settings: ${String(err)}`)
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [])

  const save = async (event: React.FormEvent) => {
    event.preventDefault()
    if (validation || saving || !hasChanges) return

    setSaving(true)
    setError('')
    setStatus('')
    try {
      const value = await invoke<UserProfile>('save_user_profile', {
        profile: { display_name: displayName },
      })
      applyProfile(value)
      setStatus(
        value.display_name
          ? `Saved. A.R.K.A will greet you as ${value.display_name}.`
          : 'Saved. A.R.K.A will use a general greeting.',
      )
    } catch (err) {
      setError(`Unable to save your name: ${String(err)}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <section className="settings-panel" aria-labelledby="personalisation-settings-heading">
      <header className="settings-section-head">
        <h2 id="personalisation-settings-heading">Personalisation</h2>
        <p>Choose how A.R.K.A greets you. Your name stays in the local application database on this device.</p>
      </header>

      {loading ? (
        <p role="status">Loading personalisation settings…</p>
      ) : profile ? (
        <form className="settings-stack profile-settings-form" onSubmit={(event) => void save(event)}>
          <label className="settings-field profile-name-field" htmlFor="profile-display-name">
            <span>Your name</span>
            <input
              id="profile-display-name"
              className="settings-input profile-name-input"
              type="text"
              autoComplete="name"
              maxLength={MAX_DISPLAY_NAME_LENGTH}
              placeholder="What should A.R.K.A call you?"
              value={displayName}
              disabled={saving}
              aria-describedby="profile-name-help profile-name-validation"
              onChange={(event) => {
                setDisplayName(event.target.value)
                setStatus('')
              }}
            />
            <span id="profile-name-help" className="settings-help-text">
              Leave this blank to use the standard time-of-day greeting.
            </span>
          </label>
          <p id="profile-name-validation" className="settings-status is-error" aria-live="polite">
            {validation}
          </p>
          <div className="settings-actions">
            <button className="btn-primary" type="submit" disabled={saving || Boolean(validation) || !hasChanges}>
              {saving ? 'Saving…' : 'Save personalisation'}
            </button>
            <p role="status" className="settings-status">{status}</p>
          </div>
        </form>
      ) : (
        <button type="button" className="btn-secondary" onClick={() => void load()}>
          Retry loading personalisation
        </button>
      )}

      {error ? <p role="alert" className="settings-status is-error">{error}</p> : null}
    </section>
  )
}
