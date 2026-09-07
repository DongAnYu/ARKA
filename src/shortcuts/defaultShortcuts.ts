import { commands, type CommandId } from '../commands/commands'

export type ShortcutScope = 'question-review'

export type ShortcutDefinition = {
  command: CommandId
  scope: ShortcutScope
  key: string
  display: string
  label: string
  allowInEditable?: boolean
  ctrl?: boolean
  alt?: boolean
  meta?: boolean
  shift?: boolean
}

export const defaultShortcuts: readonly ShortcutDefinition[] = [
  {
    command: commands.reviewPrevious,
    scope: 'question-review',
    key: 'ArrowLeft',
    display: '←',
    label: 'Previous question',
  },
  {
    command: commands.reviewNext,
    scope: 'question-review',
    key: 'ArrowRight',
    display: '→',
    label: 'Next question',
  },
  {
    command: commands.reviewDiscard,
    scope: 'question-review',
    key: 'd',
    display: 'D',
    label: 'Discard question',
  },
  {
    command: commands.reviewKeep,
    scope: 'question-review',
    key: 'k',
    display: 'K',
    label: 'Keep question',
  },
]

export function getShortcut(command: CommandId) {
  return defaultShortcuts.find((shortcut) => shortcut.command === command)
}

export function getAriaKeyShortcut(command: CommandId) {
  const shortcut = getShortcut(command)
  if (!shortcut) {
    return undefined
  }

  const modifiers = [
    shortcut.ctrl ? 'Control' : '',
    shortcut.alt ? 'Alt' : '',
    shortcut.meta ? 'Meta' : '',
    shortcut.shift ? 'Shift' : '',
  ].filter(Boolean)

  return [...modifiers, shortcut.key.length === 1 ? shortcut.key.toUpperCase() : shortcut.key].join('+')
}
