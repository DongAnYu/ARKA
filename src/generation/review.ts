import type { ChunkPreview, GenerationSummary } from './types'

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

export { answerOptions }
