import { useCallback, useEffect, useRef, useState } from 'react'

type EditableTextProps = {
  value: string
  onSave: (nextValue: string) => Promise<void>
  disabled?: boolean
  placeholder?: string
  className?: string
  inputClassName?: string
  as?: 'span' | 'p' | 'h2'
  allowEmpty?: boolean
  'aria-label'?: string
}

export function EditableText({
  value,
  onSave,
  disabled = false,
  placeholder,
  className,
  inputClassName,
  as: Tag = 'span',
  allowEmpty = false,
  'aria-label': ariaLabel,
}: EditableTextProps) {
  const [isEditing, setIsEditing] = useState(false)
  const [draft, setDraft] = useState(value)
  const [isSaving, setIsSaving] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const savingRef = useRef(false)

  useEffect(() => {
    setDraft(value)
  }, [value])

  useEffect(() => {
    if (isEditing && inputRef.current) {
      inputRef.current.focus()
      inputRef.current.select()
    }
  }, [isEditing])

  const save = useCallback(async () => {
    if (savingRef.current) {
      return
    }

    const trimmed = draft.trim()

    if (!allowEmpty && trimmed.length === 0) {
      setDraft(value)
      setIsEditing(false)
      return
    }

    if (trimmed === value.trim()) {
      setIsEditing(false)
      return
    }

    savingRef.current = true
    setIsSaving(true)

    try {
      await onSave(trimmed)
    } catch {
      setDraft(value)
    } finally {
      savingRef.current = false
      setIsSaving(false)
      setIsEditing(false)
    }
  }, [draft, value, allowEmpty, onSave])

  const cancel = useCallback(() => {
    setDraft(value)
    setIsEditing(false)
  }, [value])

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      event.currentTarget.blur()
    } else if (event.key === 'Escape') {
      event.preventDefault()
      cancel()
    }
  }

  const startEditing = () => {
    if (disabled || isSaving) {
      return
    }

    setDraft(value)
    setIsEditing(true)
  }

  if (isEditing) {
    return (
      <input
        ref={inputRef}
        type="text"
        className={`editable-text-input ${inputClassName ?? ''}`}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => void save()}
        onKeyDown={handleKeyDown}
        disabled={disabled || isSaving}
        placeholder={placeholder}
        aria-label={ariaLabel}
      />
    )
  }

  return (
    <Tag
      className={`editable-text ${className ?? ''} ${disabled ? 'is-disabled' : ''}`}
      onClick={startEditing}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          startEditing()
        }
      }}
      role="button"
      tabIndex={disabled ? -1 : 0}
      aria-label={ariaLabel ?? (placeholder ? `Edit: ${value || placeholder}` : `Edit: ${value}`)}
    >
      {value || <span className="editable-text-placeholder">{placeholder ?? ''}</span>}
    </Tag>
  )
}
