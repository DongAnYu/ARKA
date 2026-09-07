import { useEffect, useRef } from 'react'
import type { CommandId } from '../commands/commands'
import {
  defaultShortcuts,
  type ShortcutDefinition,
  type ShortcutScope,
} from './defaultShortcuts'

type ShortcutHandler = () => boolean | void

type UseKeyboardShortcutsOptions = {
  scope: ShortcutScope
  handlers: Partial<Record<CommandId, ShortcutHandler>>
  enabled?: boolean
}

function isEditableTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    Boolean(target.closest('input, textarea, select, [contenteditable="true"]'))
  )
}

function matchesShortcut(event: KeyboardEvent, shortcut: ShortcutDefinition) {
  const eventKey = event.key.length === 1 ? event.key.toLowerCase() : event.key

  return (
    eventKey === shortcut.key &&
    event.ctrlKey === Boolean(shortcut.ctrl) &&
    event.altKey === Boolean(shortcut.alt) &&
    event.metaKey === Boolean(shortcut.meta) &&
    event.shiftKey === Boolean(shortcut.shift)
  )
}

export function useKeyboardShortcuts({
  scope,
  handlers,
  enabled = true,
}: UseKeyboardShortcutsOptions) {
  const handlersRef = useRef(handlers)

  useEffect(() => {
    handlersRef.current = handlers
  }, [handlers])

  useEffect(() => {
    if (!enabled) {
      return
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) {
        return
      }

      for (const shortcut of defaultShortcuts) {
        if (shortcut.scope !== scope || !matchesShortcut(event, shortcut)) {
          continue
        }

        if (!shortcut.allowInEditable && isEditableTarget(event.target)) {
          return
        }

        const handler = handlersRef.current[shortcut.command]
        if (!handler) {
          return
        }

        const handled = handler()
        if (handled !== false) {
          event.preventDefault()
        }
        return
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [enabled, scope])
}
