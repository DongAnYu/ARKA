import type {
  ChunkPreview,
  GeneratedItem,
  GeneratedLearningItemSaveInput,
  GenerationSummary,
} from './types'

export type ReviewDecision = 'pending' | 'kept' | 'discarded'

export type ReviewQuestionDraft = {
  reviewId: string
  question: string
  option_a: string
  option_b: string
  option_c: string
  option_d: string
  correct_answer: string
  explanation: string | null
  decision: ReviewDecision
}

export type ReviewLearningItemDraft = GeneratedLearningItemSaveInput & {
  decision: ReviewDecision
}

const answerOptions = ['A', 'B', 'C', 'D'] as const

function normalizeCorrectAnswer(answer: string) {
  const normalized = answer.trim().toUpperCase()
  return answerOptions.includes(normalized as (typeof answerOptions)[number])
    ? normalized
    : 'A'
}

export function createReviewQuestionDraftsFromPreviews(
  previews: ChunkPreview[],
): ReviewQuestionDraft[] {
  return previews.flatMap((chunk) =>
    chunk.llm_result.questions.map((question, questionIndex) => ({
      reviewId: `${chunk.note_path}:${chunk.section_index}:${chunk.chunk_index}:${chunk.start_line}:${chunk.end_line}:${chunk.heading}:${questionIndex}:${question.question}`,
      question: question.question,
      option_a: question.option_a,
      option_b: question.option_b,
      option_c: question.option_c,
      option_d: question.option_d,
      correct_answer: normalizeCorrectAnswer(question.correct_answer),
      explanation: question.explanation || null,
      decision: 'pending',
    })),
  )
}

export function createReviewQuestionDrafts(
  summary: GenerationSummary,
): ReviewQuestionDraft[] {
  return createReviewQuestionDraftsFromPreviews(summary.chunk_previews)
}

export function createReviewLearningItemDraftsFromPreviews(
  previews: ChunkPreview[],
): ReviewLearningItemDraft[] {
  return previews.flatMap((chunk) =>
    chunk.llm_result.items.map((draft) => ({
      draft_id: draft.draft_id,
      content: draft.content,
      decision: 'pending',
    })),
  )
}

export function createReviewLearningItemDrafts(
  summary: GenerationSummary,
): ReviewLearningItemDraft[] {
  return createReviewLearningItemDraftsFromPreviews(summary.chunk_previews)
}

export function getReviewQuestionError(question: ReviewQuestionDraft) {
  if (!question.question.trim()) {
    return 'Add question text before keeping this question.'
  }

  const options = [
    question.option_a,
    question.option_b,
    question.option_c,
    question.option_d,
  ]

  if (options.some((option) => !option.trim())) {
    return 'Complete all four answer options before keeping this question.'
  }

  if (!answerOptions.includes(question.correct_answer as (typeof answerOptions)[number])) {
    return 'Choose which answer is correct before keeping this question.'
  }

  return ''
}

export function getReviewLearningItemError(item: ReviewLearningItemDraft) {
  const { content } = item

  if (!content.target.trim()) {
    return 'Add a learning target before keeping this item.'
  }

  if (!content.answer.trim()) {
    return 'Add the shared answer before keeping this item.'
  }

  if (!content.flashcard.prompt.trim()) {
    return 'Add a flashcard prompt before keeping this item.'
  }

  if (!content.mcq) {
    if (!content.mcq_omission_reason?.trim()) {
      return 'Explain why multiple choice is unsuitable for this item.'
    }
    return ''
  }

  if (content.mcq_omission_reason !== null) {
    return 'Remove the MCQ omission reason when a multiple-choice variant exists.'
  }

  if (content.mcq.prompt !== null && !content.mcq.prompt.trim()) {
    return 'Add an MCQ prompt, or reuse the flashcard prompt.'
  }

  if (content.mcq.distractors.length !== 3) {
    return 'Add exactly three MCQ distractors.'
  }

  const normalizedOptions = [content.answer, ...content.mcq.distractors].map(
    (option) => option.trim().toLowerCase(),
  )

  if (normalizedOptions.some((option) => !option)) {
    return 'Complete the shared answer and all three MCQ distractors.'
  }

  if (new Set(normalizedOptions).size !== normalizedOptions.length) {
    return 'The correct answer and distractors must all be different.'
  }

  return ''
}

export function prepareLearningItemForSave(
  item: ReviewLearningItemDraft,
): GeneratedLearningItemSaveInput {
  const content: GeneratedItem = {
    ...item.content,
    target: item.content.target.trim(),
    answer: item.content.answer.trim(),
    explanation: item.content.explanation?.trim() || null,
    flashcard: {
      prompt: item.content.flashcard.prompt.trim(),
    },
    mcq: item.content.mcq
      ? {
          prompt: item.content.mcq.prompt?.trim() || null,
          distractors: item.content.mcq.distractors.map((distractor) =>
            distractor.trim(),
          ),
        }
      : null,
    mcq_omission_reason: item.content.mcq
      ? null
      : item.content.mcq_omission_reason?.trim() || null,
  }

  return {
    draft_id: item.draft_id,
    content,
  }
}

export { answerOptions }
