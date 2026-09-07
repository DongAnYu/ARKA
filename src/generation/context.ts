import { createContext, useContext } from 'react'
import type {
  GenerationMode,
  GenerationProgress,
  GenerationSummary,
  Note,
} from './types'

export type GenerationContextValue = {
  vaultPath: string
  notes: Note[]
  selectedNote: Note | null
  generationMode: GenerationMode | null
  generationProgress: GenerationProgress | null
  generationSummary: GenerationSummary | null
  generationError: string
  isGenerating: boolean
  setSourceNotes: (vaultPath: string, notes: Note[]) => void
  selectNote: (note: Note) => void
  clearSelectedNote: () => void
  setGenerationMode: (mode: GenerationMode) => void
  startGeneration: () => Promise<void>
  togglePauseGeneration: () => Promise<void>
  cancelGeneration: () => Promise<void>
}

export const GenerationContext = createContext<GenerationContextValue | null>(null)

export function useGeneration() {
  const context = useContext(GenerationContext)

  if (!context) {
    throw new Error('useGeneration must be used within GenerationProvider')
  }

  return context
}
