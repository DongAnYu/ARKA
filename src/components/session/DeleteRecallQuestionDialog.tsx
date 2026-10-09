import { useEffect, useRef } from 'react'

export function DeleteRecallQuestionDialog({ prompt, deleting, error, onCancel, onDelete }: {
  prompt: string
  deleting: boolean
  error: string
  onCancel: () => void
  onDelete: () => void
}) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const cancelButton = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) return
    dialog.showModal()
    cancelButton.current?.focus()
    return () => { if (dialog.open) dialog.close() }
  }, [])

  return (
    <dialog ref={dialogRef} className="delete-space-modal session-delete-dialog" role="alertdialog"
      aria-labelledby="session-delete-heading" aria-describedby="session-delete-description"
      aria-busy={deleting} aria-modal="true" tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key !== 'Tab') return
        const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')]
        const first = buttons[0]
        const last = buttons[buttons.length - 1]
        if (!first || !last) {
          event.preventDefault()
          event.currentTarget.focus()
        } else if (event.shiftKey && document.activeElement === first) {
          event.preventDefault()
          last.focus()
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault()
          first.focus()
        }
      }}
      onCancel={(event) => { event.preventDefault(); if (!deleting) onCancel() }}>
      <h2 id="session-delete-heading">Delete question?</h2>
      <p className="session-delete-prompt">{prompt}</p>
      <p id="session-delete-description">
        This permanently deletes the learning item and all its linked MCQ and flashcard questions from your Library and future recall sessions. This cannot be undone.
      </p>
      <p>Any saved reviews still count toward today’s progress. Deleting an unanswered question does not count as a review.</p>
      {error ? <div className="error-banner" role="alert">{error}</div> : null}
      <div className="delete-space-modal-actions">
        <button ref={cancelButton} type="button" className="btn-secondary delete-space-cancel-btn"
          disabled={deleting} onClick={onCancel}>Cancel</button>
        <button type="button" className="btn-primary delete-space-confirm-btn"
          disabled={deleting} onClick={onDelete}>{deleting ? 'Deleting…' : 'Delete permanently'}</button>
      </div>
    </dialog>
  )
}
