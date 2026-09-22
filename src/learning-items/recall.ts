import type { LearningItem, LearningItemVariant, ReviewResponse, ReviewSubmission } from './types'

type SessionParent = Pick<LearningItem, 'id' | 'answer' | 'explanation'>
// Parent and variant IDs have separate meanings; never overwrite one with the other.
export type RecallItem = Omit<SessionParent, 'id'> & {
  learningItemId: number
  model: LearningItem['generation']['model']
} & LearningItemVariant

export function selectRecallVariant(item: LearningItem): RecallItem | null {
  const mcq = item.variants.find((variant) => variant.format === 'mcq')
  const flashcard = item.variants.find((variant) => variant.format === 'flashcard')
  const variant = mcq && flashcard
    ? (item.id + item.schedule.repetitions) % 2 === 0 ? flashcard : mcq
    : mcq ?? flashcard
  if (!variant) return null
  return { ...variant, learningItemId: item.id, answer: item.answer, explanation: item.explanation, model: item.generation.model }
}

export function buildReviewSubmission(item: RecallItem, response: ReviewResponse): ReviewSubmission {
  if (response.format !== item.format) throw new Error('Response does not match the displayed format')
  if (item.format === 'mcq' && response.format === 'mcq' &&
      !item.options.some((option) => option.id === response.selected_option_id)) {
    throw new Error('Choose an answer option')
  }
  return {
    learning_item_id: item.learningItemId,
    variant_id: item.id,
    response: response.format === 'mcq'
      ? { format: 'mcq', selected_option_id: response.selected_option_id }
      : { format: 'flashcard', rating: response.rating },
  }
}

export function wasRecalled(item: RecallItem, response: ReviewResponse): boolean {
  return item.format === 'mcq' && response.format === 'mcq'
    ? response.selected_option_id === item.correct_option_id
    : response.format === 'flashcard' && response.rating !== 'again'
}

export type RecallResult = { learningItemId: number; recalled: boolean }

export function summarizeRecall(results: RecallResult[]) {
  const recalled = results.filter((result) => result.recalled).length
  return { reviewed: results.length, recalled, again: results.length - recalled }
}

type ReviewSnapshot = {
  revealed: boolean
  status: 'idle' | 'submitting' | 'reviewed'
  response: ReviewResponse | null
  error: string
  nextReviewDays: number | null
}

/** One controller per displayed parent. The synchronous guard also catches clicks before React renders. */
export function createReviewController(item: RecallItem, save: (submission: ReviewSubmission) => Promise<LearningItem>) {
  let snapshot: ReviewSnapshot = { revealed: false, status: 'idle', response: null, error: '', nextReviewDays: null }
  const listeners = new Set<() => void>()
  const update = (change: Partial<ReviewSnapshot>) => {
    snapshot = { ...snapshot, ...change }
    listeners.forEach((listener) => listener())
  }
  return {
    getSnapshot: () => snapshot,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    reveal: () => update({ revealed: true }),
    async submit(response: ReviewResponse): Promise<RecallResult | null> {
      if (snapshot.status !== 'idle' || (item.format === 'flashcard' && !snapshot.revealed)) return null
      update({ status: 'submitting', error: '' })
      try {
        const reviewed = await save(buildReviewSubmission(item, response))
        update({ status: 'reviewed', response, nextReviewDays: reviewed.schedule.interval_days })
        return { learningItemId: item.learningItemId, recalled: wasRecalled(item, response) }
      } catch (error) {
        update({ status: 'idle', error: `Review was not saved. ${error instanceof Error ? error.message : String(error)} Try again.` })
        return null
      }
    },
  }
}

export function reviewIntervalLabel(days: number): string {
  return `${days} ${days === 1 ? 'day' : 'days'}`
}
