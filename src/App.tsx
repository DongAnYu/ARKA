import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import './App.css'

type ViewMode = 'reader' | 'raw'

type Note = {
  id: number | null
  path: string
  title: string
  content: string
  last_modified: string
}

type NoteGenerationReport = {
  note_path: string
  note_title: string
  total_chunks: number
}

type ChunkPreview = {
  note_path: string
  note_title: string
  heading: string
  section_index: number
  chunk_index: number
  start_line: number
  end_line: number
  char_count: number
  preview_text: string
}

type GenerationSummary = {
  total_notes: number
  total_chunks: number
  notes_with_chunks: number
  note_reports: NoteGenerationReport[]
  chunk_previews: ChunkPreview[]
}

function App() {
  const [vaultPath, setVaultPath] = useState('')
  const [notes, setNotes] = useState<Note[]>([])
  const [selectedNote, setSelectedNote] = useState<Note | null>(null)
  const [viewMode, setViewMode] = useState<ViewMode>('reader')
  const [isLoading, setIsLoading] = useState(false)
  const [isGenerating, setIsGenerating] = useState(false)
  const [generationSummary, setGenerationSummary] = useState<GenerationSummary | null>(null)
  const [showChunks, setShowChunks] = useState(false)
  const [error, setError] = useState('')

  const chooseVault = async () => {
    setError('')

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: 'Choose Obsidian Vault',
      })

      if (typeof selected !== 'string') {
        return
      }

      setVaultPath(selected)
      await loadNotes(selected)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to choose vault'
      setError(message)
    }
  }

  const loadNotes = async (path: string) => {
    setIsLoading(true)
    setError('')
    setSelectedNote(null)
    setGenerationSummary(null)
    setShowChunks(false)

    try {
      const data = await invoke<Note[]>('get_notes', { vaultPath: path })
      setNotes(data)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load notes'
      setError(message)
    } finally {
      setIsLoading(false)
    }
  }

  const generatePreview = async () => {
    if (!vaultPath) {
      setError('Choose a vault first before generating preview.')
      return
    }

    setIsGenerating(true)
    setError('')

    try {
      const summary = await invoke<GenerationSummary>('preview_generation', {
        vaultPath,
      })
      setGenerationSummary(summary)
      setShowChunks(false)
    } catch (err) {
      const message =
        err instanceof Error ? err.message : 'Failed to generate chunk preview'
      setError(message)
    } finally {
      setIsGenerating(false)
    }
  }

  return (
    <div className="app-container">
      <div className="app-header">
        <h1>Active Recall</h1>
        <div className="header-actions">
          <button
            type="button"
            className="btn-primary"
            onClick={chooseVault}
            disabled={isLoading || isGenerating}
          >
            {isLoading ? 'Loading...' : 'Choose Vault'}
          </button>
          <button
            type="button"
            className="btn-secondary"
            onClick={generatePreview}
            disabled={isLoading || isGenerating || !vaultPath}
          >
            {isGenerating ? 'Generating...' : 'Generate Preview'}
          </button>
        </div>
      </div>

      {vaultPath && <p className="vault-path">{vaultPath}</p>}
      {error && <div className="error-banner">{error}</div>}

      {generationSummary && (
        <section className="generation-summary" aria-live="polite">
          <div className="generation-summary-head">
            <h2>Chunk Preview Summary</h2>
            <button
              type="button"
              className="view-chunks-btn"
              onClick={() => setShowChunks((prev) => !prev)}
              aria-expanded={showChunks}
            >
              <svg
                className="view-chunks-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" />
                <circle cx="12" cy="12" r="3" />
              </svg>
              {showChunks ? 'Hide Chunks' : 'View Chunks'}
            </button>
          </div>
          <div className="summary-grid">
            <p>
              <span>Total Notes</span>
              <strong>{generationSummary.total_notes}</strong>
            </p>
            <p>
              <span>Notes With Chunks</span>
              <strong>{generationSummary.notes_with_chunks}</strong>
            </p>
            <p>
              <span>Total Chunks</span>
              <strong>{generationSummary.total_chunks}</strong>
            </p>
          </div>
          {showChunks && (
            <div className="chunk-panel">
              {generationSummary.chunk_previews.length === 0 ? (
                <p className="chunk-empty">No chunks were generated for this vault.</p>
              ) : (
                <ul className="chunk-list">
                  {generationSummary.chunk_previews.map((chunk) => (
                    <li
                      key={`${chunk.note_path}-${chunk.section_index}-${chunk.chunk_index}`}
                      className="chunk-item"
                    >
                      <div className="chunk-item-head">
                        <h3>{chunk.heading}</h3>
                        <span>{chunk.char_count} chars</span>
                      </div>
                      <p className="chunk-meta">
                        {chunk.note_title} • lines {chunk.start_line}-{chunk.end_line}
                      </p>
                      <p className="chunk-preview-text">{chunk.preview_text}</p>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </section>
      )}

      {selectedNote ? (
        <article className="note-detail">
          <button
            type="button"
            className="btn-back"
            onClick={() => setSelectedNote(null)}
          >
            ← Back to notes
          </button>
          <div className="note-detail-header">
            <h2 className="note-detail-title">{selectedNote.title}</h2>
            <div className="view-toggle" role="group" aria-label="View mode">
              <button
                type="button"
                className={`view-toggle-btn${viewMode === 'reader' ? ' is-active' : ''}`}
                onClick={() => setViewMode('reader')}
                aria-pressed={viewMode === 'reader'}
                aria-label="Reading view"
                title="Reading view"
              >
                <svg
                  className="view-toggle-icon"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="M12 7v14" />
                  <path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z" />
                </svg>
              </button>
              <button
                type="button"
                className={`view-toggle-btn${viewMode === 'raw' ? ' is-active' : ''}`}
                onClick={() => setViewMode('raw')}
                aria-pressed={viewMode === 'raw'}
                aria-label="Source view"
                title="Source view"
              >
                <svg
                  className="view-toggle-icon"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  aria-hidden="true"
                >
                  <path d="m16 18 6-6-6-6" />
                  <path d="m8 6-6 6 6 6" />
                </svg>
              </button>
            </div>
          </div>
          {viewMode === 'reader' ? (
            <div className="note-reader">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {selectedNote.content}
              </ReactMarkdown>
            </div>
          ) : (
            <pre className="note-detail-content">{selectedNote.content}</pre>
          )}
        </article>
      ) : (
        <div className="notes-container" aria-live="polite">
          {notes.length === 0 ? (
            <p className="empty-state">Choose a vault to load your notes.</p>
          ) : (
            <>
              <p className="notes-count">Found {notes.length} Notes</p>
              <ul className="note-list">
                {notes.map((note) => (
                  <li key={note.path}>
                    <button
                      type="button"
                      className="note-item"
                      onClick={() => {
                        setViewMode('reader')
                        setSelectedNote(note)
                      }}
                    >
                      {note.title}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </div>
  )
}

export default App
