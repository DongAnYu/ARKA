import type { GenerationMetadata, SourceReference } from '../generation/types'

export type McqOption = {
  id: string
  text: string
}

export type ReviewRating = 'again' | 'hard' | 'good' | 'easy'
export type ReviewIntervals = Record<ReviewRating, number>
export type ReviewResponse =
  | { format: 'mcq'; selected_option_id: string }
  | { format: 'flashcard'; rating: ReviewRating }
export type ReviewSubmission = {
  learning_item_id: number
  variant_id: number
  response: ReviewResponse
}

export type LearningItemVariant =
  | {
      format: 'mcq'
      id: number
      prompt: string
      options: McqOption[]
      correct_option_id: string
    }
  | {
      format: 'flashcard'
      id: number
      prompt: string
    }

export type ReviewState = {
  repetitions: number
  interval_days: number
  ease_factor: number
  next_review_at: string | null
  last_reviewed_at: string | null
}

export type LearningItem = {
  id: number
  target: string | null
  answer: string | null
  explanation: string | null
  space_id: number
  generation: GenerationMetadata
  source: SourceReference | null
  recall_state: 'new' | 'scheduled'
  schedule: ReviewState
  variants: LearningItemVariant[]
}

export type LearningItemVariantInput =
  | {
      format: 'mcq'
      prompt: string
      options: McqOption[]
      correct_option_id: string
    }
  | {
      format: 'flashcard'
      prompt: string
    }

export type LearningItemEditInput = {
  target: string | null
  answer: string
  explanation: string | null
  space_id: number
  variants: LearningItemVariantInput[]
}
