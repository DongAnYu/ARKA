import { ArrowLeft } from 'lucide-react'
import { Link } from 'react-router-dom'

type BackToHomeProps = {
  disabled?: boolean
  onActivate?: () => void
}

export function BackToHome({ disabled = false, onActivate }: BackToHomeProps) {
  const content = (
    <>
      <ArrowLeft className="size-4" aria-hidden="true" />
      Back to home
    </>
  )

  if (onActivate) {
    return (
      <button
        type="button"
        className="btn-back page-back-link"
        onClick={onActivate}
        disabled={disabled}
      >
        {content}
      </button>
    )
  }

  return (
    <Link className="btn-back page-back-link" to="/">
      {content}
    </Link>
  )
}
