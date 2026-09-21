export const commands = {
  reviewPrevious: 'review.previous',
  reviewNext: 'review.next',
  reviewDiscard: 'review.discard',
  reviewKeep: 'review.keep',
  recallChoose1: 'recall.choose.1',
  recallChoose2: 'recall.choose.2',
  recallChoose3: 'recall.choose.3',
  recallChoose4: 'recall.choose.4',
  recallPrimaryAction: 'recall.primary-action',
} as const

export type CommandId = (typeof commands)[keyof typeof commands]

export const recallChoiceCommands = [
  commands.recallChoose1,
  commands.recallChoose2,
  commands.recallChoose3,
  commands.recallChoose4,
] as const
