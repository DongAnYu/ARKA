export const commands = {
  reviewPrevious: 'review.previous',
  reviewNext: 'review.next',
  reviewDiscard: 'review.discard',
  reviewKeep: 'review.keep',
} as const

export type CommandId = (typeof commands)[keyof typeof commands]
