// Node 22.18+ runs the isolated TypeScript logic without another test framework.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { buildReviewSubmission, createReviewController, selectRecallVariant, summarizeRecall, reviewIntervalLabel } from '../src/learning-items/recall.ts'

const mcq = { id: 101, format: 'mcq', prompt: 'Which complexity?', options: [
  { id: 'stable-correct', text: 'O(log n)' }, { id: 'wrong', text: 'O(n)' },
], correct_option_id: 'stable-correct' }
const flashcard = { id: 102, format: 'flashcard', prompt: 'Recall the complexity.' }
function parent(variants, overrides = {}) {
  return { id: 2, target: null, answer: 'O(log n)', explanation: null, status: 'ready',
    generation: { model: null }, schedule: { repetitions: 0 }, variants, ...overrides }
}
const mcqResponse = { format: 'mcq', selected_option_id: 'stable-correct' }
const flashResponse = { format: 'flashcard', rating: 'good' }
const savedItem = { schedule: { interval_days: 6 } }

test('legacy MCQ-only selects MCQ without inventing target or flashcard', () => {
  const input = parent([mcq])
  const original = structuredClone(input)
  assert.equal(selectRecallVariant(input).format, 'mcq')
  assert.deepEqual(input, original)
})

test('flashcard-only uses shared answer and optional explanation', () => {
  const item = selectRecallVariant(parent([flashcard]))
  assert.equal(item.format, 'flashcard')
  assert.equal(item.answer, 'O(log n)')
  assert.equal(item.explanation, null)
})

test('paired selection is deterministic, independent of order, and alternates by repetitions', () => {
  for (const id of [1, 2, 3]) {
    for (let repetitions = 0; repetitions < 4; repetitions++) {
      const input = parent([mcq, flashcard], { id, schedule: { repetitions } })
      const selected = selectRecallVariant(input)
      assert.equal(selected.format, (id + repetitions) % 2 === 0 ? 'flashcard' : 'mcq')
      assert.deepEqual(selected, selectRecallVariant({ ...input, variants: [flashcard, mcq] }))
      assert.deepEqual(selected, selectRecallVariant(input))
      assert.equal(selected.learningItemId, id)
    }
  }
})

test('repair items and missing variants are excluded', () => {
  assert.equal(selectRecallVariant(parent([mcq], { status: 'needs_repair' })), null)
  assert.equal(selectRecallVariant(parent([])), null)
})

test('MCQ payload has only parent ID, variant ID, and selected option', () => {
  assert.deepEqual(buildReviewSubmission(selectRecallVariant(parent([mcq])), {
    ...mcqResponse, is_correct: true, rating: 'easy', schedule: {},
  }), { learning_item_id: 2, variant_id: 101, response: mcqResponse })
})

test('flashcard payload has only parent ID, variant ID, and rating', () => {
  assert.deepEqual(buildReviewSubmission(selectRecallVariant(parent([flashcard])), {
    ...flashResponse, answer: 'O(log n)', repetitions: 2,
  }), { learning_item_id: 2, variant_id: 102, response: flashResponse })
})

test('wrong format and nonexistent MCQ options are rejected before IPC', () => {
  const item = selectRecallVariant(parent([mcq]))
  assert.throws(() => buildReviewSubmission(item, flashResponse))
  assert.throws(() => buildReviewSubmission(item, { ...mcqResponse, selected_option_id: 'missing' }))
})

test('flashcard cannot be rated before reveal; new parent starts unrevealed', async () => {
  let calls = 0
  const save = async () => { calls++; return savedItem }
  const item = selectRecallVariant(parent([flashcard]))
  const controller = createReviewController(item, save)
  assert.equal(await controller.submit(flashResponse), null)
  assert.equal(calls, 0)
  controller.reveal()
  assert.deepEqual(await controller.submit(flashResponse), { learningItemId: 2, recalled: true })
  assert.equal(calls, 1)
  const next = createReviewController({ ...item, learningItemId: 3 }, save)
  assert.deepEqual(next.getSnapshot(), { revealed: false, status: 'idle', response: null, error: '', nextReviewDays: null })
})

test('failed review produces no result, keeps reveal, and allows retry', async () => {
  let fail = true
  const controller = createReviewController(selectRecallVariant(parent([flashcard])), async () => {
    if (fail) throw new Error('offline')
    return savedItem
  })
  controller.reveal()
  const results = []
  const first = await controller.submit(flashResponse)
  if (first) results.push(first)
  assert.deepEqual(summarizeRecall(results), { reviewed: 0, recalled: 0, again: 0 })
  assert.equal(controller.getSnapshot().status, 'idle')
  assert.equal(controller.getSnapshot().revealed, true)
  assert.match(controller.getSnapshot().error, /offline.*Try again/)
  fail = false
  results.push(await controller.submit(flashResponse))
  assert.deepEqual(summarizeRecall(results), { reviewed: 1, recalled: 1, again: 0 })
  assert.equal(controller.getSnapshot().error, '')
})

for (const format of ['mcq', 'flashcard']) {
  test(`${format} prevents concurrent and already-successful duplicate submissions`, async () => {
    const { promise, resolve } = Promise.withResolvers()
    let calls = 0
    const item = selectRecallVariant(parent([format === 'mcq' ? mcq : flashcard]))
    const controller = createReviewController(item, () => { calls++; return promise })
    const response = format === 'mcq' ? mcqResponse : flashResponse
    controller.reveal()
    const pending = controller.submit(response)
    assert.equal(controller.getSnapshot().status, 'submitting')
    assert.equal(await controller.submit(response), null)
    assert.equal(calls, 1)
    resolve(savedItem)
    assert.ok(await pending)
    assert.equal(await controller.submit(response), null)
    assert.equal(calls, 1)
  })
}

test('legacy, flashcard-only, and mixed reviews produce neutral completion totals', async () => {
  const responses = [mcqResponse, { ...mcqResponse, selected_option_id: 'wrong' },
    ...['again', 'hard', 'good', 'easy'].map((rating) => ({ format: 'flashcard', rating }))]
  const results = []
  for (const [index, response] of responses.entries()) {
    const item = selectRecallVariant(parent([response.format === 'mcq' ? mcq : flashcard], { id: index + 1 }))
    const controller = createReviewController(item, async () => savedItem)
    controller.reveal()
    results.push(await controller.submit(response))
  }
  assert.deepEqual(summarizeRecall(results), { reviewed: 6, recalled: 4, again: 2 })
  assert.deepEqual(summarizeRecall(results.slice(0, 2)), { reviewed: 2, recalled: 1, again: 1 })
  assert.deepEqual(summarizeRecall(results.slice(2)), { reviewed: 4, recalled: 3, again: 1 })
  assert.deepEqual(summarizeRecall([]), { reviewed: 0, recalled: 0, again: 0 })
})

for (const format of ['mcq', 'flashcard']) {
  test(`${format} displays the saved backend interval, not a local estimate`, async () => {
    const item = selectRecallVariant(parent([format === 'mcq' ? mcq : flashcard]))
    const controller = createReviewController(item, async () => ({ schedule: { interval_days: 37 } }))
    controller.reveal()
    assert.equal(controller.getSnapshot().nextReviewDays, null)
    await controller.submit(format === 'mcq' ? mcqResponse : flashResponse)
    assert.equal(controller.getSnapshot().nextReviewDays, 37)
  })
}

test('interval labels use singular and plural days', () => {
  assert.equal(reviewIntervalLabel(1), '1 day')
  assert.equal(reviewIntervalLabel(6), '6 days')
})
