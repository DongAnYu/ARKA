import { useEffect, useState } from 'react'
import { getVersion } from '@tauri-apps/api/app'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { error as logError } from '@tauri-apps/plugin-log'
import { relaunch } from '@tauri-apps/plugin-process'
import { Download, RefreshCw } from 'lucide-react'

type UpdateCheckStatus = 'idle' | 'checking' | 'up-to-date' | 'available' | 'installing' | 'restart-needed' | 'error'

export function SettingsPage() {
  const [version, setVersion] = useState<string | null>(null)
  const [isLoadingVersion, setIsLoadingVersion] = useState(true)
  const [hasVersionLoadError, setHasVersionLoadError] = useState(false)
  const [updateCheckStatus, setUpdateCheckStatus] = useState<UpdateCheckStatus>('idle')
  const [availableUpdate, setAvailableUpdate] = useState<Update | null>(null)
  const [updateError, setUpdateError] = useState<string | null>(null)

  useEffect(() => {
    let isMounted = true

    getVersion()
      .then((currentVersion) => {
        if (!isMounted) {
          return
        }

        setVersion(currentVersion)
      })
      .catch((error) => {
        console.error('Failed to load application version:', error)

        if (isMounted) {
          setHasVersionLoadError(true)
        }
      })
      .finally(() => {
        if (isMounted) {
          setIsLoadingVersion(false)
        }
      })

    return () => {
      isMounted = false
    }
  }, [])

  const versionLabel = version ? `Version ${version}` : 'Version unavailable'
  const versionStatus = hasVersionLoadError
    ? 'Unable to load the application version.'
    : isLoadingVersion
      ? 'Loading application version.'
      : `Current version: ${versionLabel}.`
  const updateStatus = updateCheckStatus === 'checking'
    ? 'Checking for updates.'
    : updateCheckStatus === 'up-to-date'
      ? `You're up to date — ${versionLabel}.`
      : updateCheckStatus === 'available'
        ? `Version ${availableUpdate?.version} is available.`
        : updateCheckStatus === 'installing'
          ? `Downloading and installing Version ${availableUpdate?.version}. A.R.K.A will restart when it is ready.`
          : updateCheckStatus === 'restart-needed'
            ? `Version ${availableUpdate?.version} is installed. Restart A.R.K.A to finish.`
        : updateCheckStatus === 'error'
          ? updateError
          : ''

  const checkForUpdates = async () => {
    setUpdateCheckStatus('checking')
    setAvailableUpdate(null)
    setUpdateError(null)

    try {
      const update = await check()

      if (!update) {
        setUpdateCheckStatus('up-to-date')
        return
      }

      setAvailableUpdate(update)
      setUpdateCheckStatus('available')
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error)
      void logError(`Updater check failed: ${detail}`).catch((loggingError) => {
        console.error('Failed to record updater check error:', loggingError)
      })
      setUpdateError('Unable to check for updates. Try again later.')
      setUpdateCheckStatus('error')
    }
  }

  const dismissAvailableUpdate = () => {
    if (availableUpdate) {
      void availableUpdate.close().catch((error) => {
        console.error('Failed to release updater resource:', error)
      })
    }

    setAvailableUpdate(null)
    setUpdateError(null)
    setUpdateCheckStatus('idle')
  }

  const installAvailableUpdate = async () => {
    if (!availableUpdate) {
      return
    }

    setUpdateCheckStatus('installing')
    setUpdateError(null)
    let updateInstalled = false

    try {
      await availableUpdate.downloadAndInstall()
      updateInstalled = true
      setUpdateCheckStatus('restart-needed')
      await relaunch()
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error)

      if (updateInstalled) {
        void logError(`Application restart failed after update: ${detail}`).catch((loggingError) => {
          console.error('Failed to record updater restart error:', loggingError)
        })
        setUpdateError('Update installed. Restart A.R.K.A to finish.')
        setUpdateCheckStatus('restart-needed')
        return
      }

      void logError(`Updater installation failed: ${detail}`).catch((loggingError) => {
        console.error('Failed to record updater installation error:', loggingError)
      })
      setUpdateError('Unable to install the update. Try again later.')
      setUpdateCheckStatus('error')
    }
  }

  const restartApp = async () => {
    try {
      await relaunch()
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error)
      void logError(`Application restart failed after update: ${detail}`).catch((loggingError) => {
        console.error('Failed to record updater restart error:', loggingError)
      })
      setUpdateError('Update installed. Restart A.R.K.A to finish.')
      setUpdateCheckStatus('restart-needed')
    }
  }

  return (
    <div className="app-container settings-page" aria-label="Settings page">
      <header className="settings-panel settings-page-intro">
        <h1>Settings</h1>
        <p className="settings-help-text">Manage how A.R.K.A runs on this device.</p>
      </header>

      <section className="settings-panel settings-application" aria-labelledby="application-settings-heading">
        <header className="settings-section-head">
          <h2 id="application-settings-heading">Application</h2>
        </header>

        <div className="settings-application-content">
          <div className="settings-application-details">
            <p className="settings-application-name">Status</p>
            <p className="settings-application-version">{versionLabel}</p>
            <p className="settings-help-text settings-application-description">
              Keep A.R.K.A up to date with the latest features and fixes.
            </p>
            <p className="settings-application-delivery">
              Updates are signed and delivered via{' '}
              <a
                href="https://github.com/DongAnYu/ARKA/releases"
                target="_blank"
                rel="noreferrer"
              >
                https://github.com/DongAnYu/ARKA/releases
              </a>
              .
            </p>
          </div>

          <div className="settings-application-actions">
            {updateCheckStatus === 'restart-needed' ? (
              <button
                type="button"
                className="btn-primary"
                onClick={() => void restartApp()}
                aria-describedby="application-version-status application-update-status"
              >
                <RefreshCw aria-hidden="true" />
                Restart A.R.K.A
              </button>
            ) : availableUpdate ? (
              <div className="settings-update-actions" role="group" aria-label="Available update actions">
                <button
                  type="button"
                  className="btn-primary"
                  onClick={() => void installAvailableUpdate()}
                  disabled={updateCheckStatus === 'installing'}
                  aria-describedby="application-version-status application-update-status"
                >
                  <Download aria-hidden="true" />
                  {updateCheckStatus === 'installing' ? 'Updating...' : 'Update now'}
                </button>
                <button
                  type="button"
                  className="btn-secondary"
                  onClick={dismissAvailableUpdate}
                  disabled={updateCheckStatus === 'installing'}
                >
                  Later
                </button>
              </div>
            ) : (
              <button
                type="button"
                className="btn-primary"
                onClick={checkForUpdates}
                disabled={updateCheckStatus === 'checking'}
                aria-describedby="application-version-status application-update-status"
              >
                <RefreshCw aria-hidden="true" />
                {updateCheckStatus === 'checking' ? 'Checking...' : 'Check for updates'}
              </button>
            )}
            <p id="application-version-status" className="settings-status" role="status" aria-live="polite">
              {versionStatus}
            </p>
            <p id="application-update-status" className="settings-status" role="status" aria-live="polite">
              {updateStatus}
            </p>
          </div>
        </div>
      </section>
    </div>
  )
}
