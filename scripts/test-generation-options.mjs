import assert from 'node:assert/strict'
import { test } from 'node:test'
import { buildGenerationStartRequest, DEFAULT_GENERATION_OPTIONS, LEARNING_PURPOSES, maximumInputError, resolveGenerationOptions } from '../src/generation/options.ts'
import { createReviewLearningItemDrafts, getReviewLearningItemError, prepareLearningItemForSave } from '../src/generation/review.ts'

test('omitted options and omitted fields share backend defaults', () => {
  const defaults = { max_learning_items: 20, purpose: 'balanced' }
  for (const input of [undefined, null, {}]) assert.deepEqual(resolveGenerationOptions(input), defaults)
  assert.deepEqual(DEFAULT_GENERATION_OPTIONS, defaults)
  assert.deepEqual(resolveGenerationOptions({ max_learning_items: 7 }), { ...defaults, max_learning_items: 7 })
  assert.deepEqual(resolveGenerationOptions({ purpose: 'foundations' }), { ...defaults, purpose: 'foundations' })
})

test('four purposes and increased or decreased positive maximums are accepted', () => {
  for (const purpose of ['balanced', 'foundations', 'explanations', 'practical_application']) {
    for (const maximum of [1, 7, 20, 100]) {
      assert.deepEqual(resolveGenerationOptions({ purpose, max_learning_items: maximum }), {
        purpose, max_learning_items: maximum,
      })
    }
  }
})

test('invalid maximums, purposes, and options are rejected before IPC', () => {
  for (const maximum of [0, -1, 1.5, NaN, Infinity, '20', null, undefined, true, [], 4294967296]) {
    assert.throws(() => resolveGenerationOptions({ max_learning_items: maximum }))
  }
  for (const purpose of ['definitions', 'auto', 'Balanced', null, undefined, 20]) {
    assert.throws(() => resolveGenerationOptions({ purpose }))
  }
  for (const input of [[], 'invalid', { maxLearningItems: 5 }]) {
    assert.throws(() => resolveGenerationOptions(input))
  }
})

test('Default submission captures immutable settings for the complete selected note', () => {
  const settings = { max_learning_items: 7, purpose: 'foundations' }
  const request = buildGenerationStartRequest('default', '/vault', '/vault/chosen.md', settings)
  settings.max_learning_items = 100
  settings.purpose = 'explanations'
  assert.deepEqual(request, {
    command: 'start_preview_generation',
    args: { vaultPath: '/vault/chosen.md', options: { max_learning_items: 7, purpose: 'foundations' } },
  })
  assert.throws(() => { request.args.options.max_learning_items = 100 })
  assert.throws(() => buildGenerationStartRequest('default', '/vault', '/vault/chosen.md', { ...settings, max_learning_items: 0 }))
})

test('Deep thinking retains its original IPC arguments without Default options', () => {
  assert.deepEqual(buildGenerationStartRequest('graph', '/vault', '/vault/chosen.md', DEFAULT_GENERATION_OPTIONS), {
    command: 'start_graph_generation_job', args: { vaultPath: '/vault' },
  })
})

test('numeric form edits are validated before submission and purposes include their descriptions', () => {
  for (const input of ['', '0', '-1', '1.5', '2e1', 'Auto', '4294967296']) assert.ok(maximumInputError(input))
  for (const input of ['1', '20', '100', '4294967295']) assert.equal(maximumInputError(input), null)
  assert.equal(Object.keys(LEARNING_PURPOSES).length, 4)
  assert.match(LEARNING_PURPOSES.foundations.description, /definitions, prerequisites/)
  for (const value of Object.values(LEARNING_PURPOSES)) assert.ok(value.label && value.description)
})

test('selected-only learning drafts retain the existing review and save contract', () => {
  const content = { knowledge_point_id: 'kp_1', target: 'Search complexity', answer: 'O(log n)', explanation: null,
    flashcard: { prompt: 'What is the worst-case time complexity of binary search on a sorted array?' },
    mcq: { prompt: null, distractors: ['O(1)', 'O(n)', 'O(n²)'] }, mcq_omission_reason: null }
  const summary = { chunk_previews: [{ llm_result: { items: [{ draft_id: 'backend-draft', content }] } }] }
  const drafts = createReviewLearningItemDrafts(summary)
  assert.equal(drafts.length, 1) // Both variants belong to this one target.
  assert.equal(getReviewLearningItemError(drafts[0]), '')
  assert.deepEqual(prepareLearningItemForSave(drafts[0]), { draft_id: 'backend-draft', content })
  assert.equal(drafts[0].decision, 'pending')
})

test('form and IPC validation preserve count boundaries and distinct input formats', () => {
  for (const maximum of [1, 20, 4294967295]) {
    assert.equal(maximumInputError(String(maximum)), null)
    assert.equal(resolveGenerationOptions({ max_learning_items: maximum }).max_learning_items, maximum)
  }
  for (const maximum of [0, -1, 1.5, 4294967296, Infinity, NaN]) {
    assert.ok(maximumInputError(String(maximum)))
    assert.throws(() => resolveGenerationOptions({ max_learning_items: maximum }))
  }
  for (const input of [' 20', '20 ', '+20', '20.0', '2e1', '9'.repeat(400)]) {
    assert.equal(maximumInputError(input), 'Enter a positive whole number.')
  }
  assert.equal(maximumInputError('0020'), null)
  assert.equal(maximumInputError('4294967296'), 'Enter a maximum no greater than 4294967295.')
  assert.throws(() => resolveGenerationOptions({ max_learning_items: '20' }))
})

test('inherited object properties are not supported learning purposes', () => {
  for (const purpose of ['constructor', 'toString', 'hasOwnProperty', 'valueOf', '__proto__']) {
    assert.throws(() => resolveGenerationOptions({ purpose }), /Choose a supported learning purpose/)
  }
})
