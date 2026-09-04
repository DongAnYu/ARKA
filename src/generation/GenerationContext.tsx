import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'
import { invoke } from '@tauri-apps/api/core'
import { GenerationContext, type GenerationContextValue } from './context'
import type {
  GenerationMode,
  GenerationProgress,
  GenerationSummary,
  Note,
} from './types'

const POLL_INTERVAL_MS = 700

function errorMessage(error: unknown, fallback: string) {
  if (typeof error === 'string') {
    return error
  }

  return error instanceof Error ? error.message : fallback
}

export function GenerationProvider({ children }: { children: ReactNode }) {
  const [vaultPath, setVaultPath] = useState('')
  const [notes, setNotes] = useState<Note[]>([])
  const [selectedNote, setSelectedNote] = useState<Note | null>(null)
  const [generationMode, setMode] = useState<GenerationMode | null>(null)
  const [generationJobId, setGenerationJobId] = useState<string | null>(null)
  const [generationProgress, setGenerationProgress] =
    useState<GenerationProgress | null>(null)
  const [generationSummary, setGenerationSummary] =
    useState<GenerationSummary | null>(null)
  const [generationError, setGenerationError] = useState('')
  const [isStarting, setIsStarting] = useState(false)

  const isGenerating = isStarting || generationJobId !== null

  const setSourceNotes = useCallback((path: string, sourceNotes: Note[]) => {
    setVaultPath(path)
    setNotes(sourceNotes)
    setSelectedNote(sourceNotes.length === 1 ? sourceNotes[0] : null)
    setMode(null)
    setGenerationProgress(null)
    setGenerationSummary(null)
    setGenerationError('')
  }, [])

  const selectNote = useCallback((note: Note) => {
    setSelectedNote(note)
    setMode(null)
  }, [])

  const clearSelectedNote = useCallback(() => {
    setSelectedNote(null)
    setMode(null)
    setGenerationProgress(null)
    setGenerationSummary(null)
    setGenerationError('')
  }, [])

  const setGenerationMode = useCallback((mode: GenerationMode) => {
    setGenerationError('')
    setMode(mode)
  }, [])

  const startGeneration = useCallback(async () => {
    if (!vaultPath || !selectedNote) {
      setGenerationError('Choose a note before generating questions.')
      return
    }

    if (!generationMode) {
      setGenerationError('Choose a generation mode first.')
      return
    }

    setIsStarting(true)
    setGenerationError('')
    setGenerationProgress(null)
    setGenerationSummary(null)

    const command =
      generationMode === 'graph'
        ? 'start_graph_generation_job'
        : 'start_preview_generation'

    try {
      const jobId = await invoke<string>(command, { vaultPath })
      setGenerationJobId(jobId)
    } catch (error) {
      setGenerationError(errorMessage(error, 'Failed to start question generation'))
    } finally {
      setIsStarting(false)
    }
  }, [generationMode, selectedNote, vaultPath])

  useEffect(() => {
    if (!generationJobId) {
      return
    }

    let disposed = false
    let timer: number | undefined

    const poll = async () => {
      try {
        const progress = await invoke<GenerationProgress>(
          'get_preview_generation_progress',
          { jobId: generationJobId },
        )

        if (disposed) {
          return
        }

        setGenerationProgress(progress)

        if (progress.is_finished) {
          if (progress.summary) {
            setGenerationSummary(progress.summary)
          }
          setGenerationJobId(null)
          return
        }

        timer = window.setTimeout(poll, POLL_INTERVAL_MS)
      } catch (error) {
        if (disposed) {
          return
        }

        setGenerationError(
          errorMessage(error, 'Failed to load generation progress'),
        )
        setGenerationJobId(null)
      }
    }

    void poll()

    return () => {
      disposed = true
      if (timer !== undefined) {
        window.clearTimeout(timer)
      }
    }
  }, [generationJobId])

  const togglePauseGeneration = useCallback(async () => {
    if (!generationJobId || !generationProgress) {
      return
    }

    const paused = !generationProgress.is_paused

    try {
      await invoke('set_preview_generation_paused', {
        jobId: generationJobId,
        paused,
      })
      setGenerationProgress((current) =>
        current ? { ...current, is_paused: paused } : current,
      )
    } catch (error) {
      setGenerationError(
        errorMessage(error, 'Failed to toggle generation pause'),
      )
    }
  }, [generationJobId, generationProgress])

  const cancelGeneration = useCallback(async () => {
    if (!generationJobId) {
      return
    }

    try {
      await invoke('cancel_preview_generation', { jobId: generationJobId })
      setGenerationProgress(null)
      setGenerationSummary(null)
      setGenerationJobId(null)
    } catch (error) {
      setGenerationError(errorMessage(error, 'Failed to cancel generation'))
    }
  }, [generationJobId])

  const value = useMemo<GenerationContextValue>(
    () => ({
      vaultPath,
      notes,
      selectedNote,
      generationMode,
      generationProgress,
      generationSummary,
      generationError,
      isGenerating,
      setSourceNotes,
      selectNote,
      clearSelectedNote,
      setGenerationMode,
      startGeneration,
      togglePauseGeneration,
      cancelGeneration,
    }),
    [
      vaultPath,
      notes,
      selectedNote,
      generationMode,
      generationProgress,
      generationSummary,
      generationError,
      isGenerating,
      setSourceNotes,
      selectNote,
      clearSelectedNote,
      setGenerationMode,
      startGeneration,
      togglePauseGeneration,
      cancelGeneration,
    ],
  )

  return (
    <GenerationContext.Provider value={value}>
      {children}
    </GenerationContext.Provider>
  )
}
