export type LearningItemEditorValue = {
  target: string
  answer: string
  explanation: string | null
  flashcard: { prompt: string } | null
  mcq: { prompt: string; distractors: string[] } | null
  mcqOmissionReason: string | null
}

export function getLearningItemEditorError(
  value: LearningItemEditorValue,
  options: { requireMcqOmissionReason?: boolean } = {},
) {
  if (!value.answer.trim()) {
    return 'Add the shared answer before saving this item.'
  }

  if (!value.flashcard && !value.mcq) {
    return 'This learning item needs at least one practice variant.'
  }

  if (value.flashcard) {
    if (!value.target.trim()) {
      return 'Add a learning target for the flashcard.'
    }
    if (!value.flashcard.prompt.trim()) {
      return 'Add a flashcard prompt before saving this item.'
    }
  }

  if (!value.mcq) {
    if (options.requireMcqOmissionReason && !value.mcqOmissionReason?.trim()) {
      return 'Explain why multiple choice is unsuitable for this item.'
    }
    return ''
  }

  if (!value.mcq.prompt.trim()) {
    return 'Add a multiple-choice question before saving this item.'
  }

  if (value.mcq.distractors.length !== 3) {
    return 'Add exactly three MCQ distractors.'
  }

  const optionsText = [value.answer, ...value.mcq.distractors].map((option) =>
    option.trim().toLowerCase(),
  )
  if (optionsText.some((option) => !option)) {
    return 'Complete the shared answer and all three MCQ distractors.'
  }
  if (new Set(optionsText).size !== optionsText.length) {
    return 'The correct answer and distractors must all be different.'
  }

  return ''
}
