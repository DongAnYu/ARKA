import type { GenerationMode, GenerationOptions, GenerationPurpose } from './types.ts'

export const DEFAULT_GENERATION_OPTIONS: GenerationOptions = Object.freeze({
  max_learning_items: 20,
  purpose: 'balanced',
})

/** Upper bound of the backend's positive u32 count. */
export const MAX_LEARNING_ITEMS = 0xffffffff

export const LEARNING_PURPOSES: Readonly<Record<GenerationPurpose, { label: string; description: string }>> = {
  balanced: { label: 'Balanced', description: 'Broad coverage of important knowledge.' },
  foundations: { label: 'Foundations', description: 'Essential definitions, prerequisites, and core principles.' },
  explanations: { label: 'Explanations', description: 'Mechanisms, causes, relationships, and conditions.' },
  practical_application: { label: 'Practical application', description: 'Procedures, decisions, and applying knowledge.' },
}

export const DEFAULT_GENERATION_PHASES = [
  'Extracting knowledge', 'Assessing importance', 'Selecting concepts', 'Generating learning items',
]

function isPositiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0
}

export function isGenerationPurpose(value: unknown): value is GenerationPurpose {
  return typeof value === 'string' && Object.hasOwn(LEARNING_PURPOSES, value)
}

/** Keep incomplete numeric edits in the form, without sending them to IPC. */
export function maximumInputError(input: string): string | null {
  const maximum = Number(input)
  if (!/^\d+$/.test(input) || !isPositiveInteger(maximum)) {
    return 'Enter a positive whole number.'
  }
  if (maximum > MAX_LEARNING_ITEMS) return `Enter a maximum no greater than ${MAX_LEARNING_ITEMS}.`
  return null
}

/** Match the backend u32 contract and reject malformed input before IPC. */
export function resolveGenerationOptions(input?: unknown): GenerationOptions {
  if (input === undefined || input === null) {
    return Object.freeze({ ...DEFAULT_GENERATION_OPTIONS })
  }
  if (typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('Generation options must be an object.')
  }
  const fields = input as Record<string, unknown>
  if (Object.keys(fields).some((key) => key !== 'max_learning_items' && key !== 'purpose')) {
    throw new Error('Unknown generation option.')
  }
  const maximum = Object.hasOwn(fields, 'max_learning_items')
    ? fields.max_learning_items
    : DEFAULT_GENERATION_OPTIONS.max_learning_items
  const purpose = Object.hasOwn(fields, 'purpose')
    ? fields.purpose
    : DEFAULT_GENERATION_OPTIONS.purpose
  if (!isPositiveInteger(maximum) || maximum > MAX_LEARNING_ITEMS) {
    throw new Error(`The maximum must be a positive integer no greater than ${MAX_LEARNING_ITEMS}.`)
  }
  if (!isGenerationPurpose(purpose)) {
    throw new Error('Choose a supported learning purpose.')
  }
  return Object.freeze({ max_learning_items: maximum, purpose })
}

type GenerationStartRequest =
  | { command: 'start_preview_generation'; args: { vaultPath: string; options: GenerationOptions } }
  | { command: 'start_graph_generation_job'; args: { vaultPath: string } }

/** Copy Default configuration at submission; graph IPC remains unchanged. */
export function buildGenerationStartRequest(
  mode: GenerationMode,
  vaultPath: string,
  selectedNotePath: string,
  options: GenerationOptions,
): GenerationStartRequest {
  if (mode === 'graph') {
    return { command: 'start_graph_generation_job', args: { vaultPath } }
  }
  return {
    command: 'start_preview_generation',
    args: { vaultPath: selectedNotePath, options: resolveGenerationOptions(options) },
  }
}
