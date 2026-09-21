import { commands, type CommandId } from '../commands/commands'

export type ShortcutScope = 'question-review' | 'recall-session'

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
  {
    command: commands.recallChoose1,
    scope: 'recall-session',
    key: '1',
    display: '1',
    label: 'Choose answer or rating 1',
  },
  {
    command: commands.recallChoose2,
    scope: 'recall-session',
    key: '2',
    display: '2',
    label: 'Choose answer or rating 2',
  },
  {
    command: commands.recallChoose3,
    scope: 'recall-session',
    key: '3',
    display: '3',
    label: 'Choose answer or rating 3',
  },
  {
    command: commands.recallChoose4,
    scope: 'recall-session',
    key: '4',
    display: '4',
    label: 'Choose answer or rating 4',
  },
  {
    command: commands.recallPrimaryAction,
    scope: 'recall-session',
    key: 'Enter',
    display: 'Enter',
    label: 'Reveal, submit, or continue',
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
