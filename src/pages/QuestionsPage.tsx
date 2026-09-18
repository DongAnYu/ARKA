import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowRight, ChevronDown, Ellipsis, Trash2 } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { BackToHome } from '../components/BackToHome'
import { RecallDonutChart } from '../components/RecallDonutChart'
import { LearningItemEditor } from '../components/LearningItemEditor'
import {
  getLearningItemEditorError,
  type LearningItemEditorValue,
} from '../learning-items/editor'
import type {
  LearningItem,
  LearningItemEditInput,
  LearningItemVariantInput,
} from '../learning-items/types'

type RecallSpace = {
  id: number
  name: string
  description: string | null
}

type RecallSpaceSummary = {
  id: number
  total_questions: number
  due_count: number
  overdue_count: number
  new_count: number
}

type RecallDashboard = {
  new_count: number
  spaces: RecallSpaceSummary[]
}

const learningItemLabel = (count: number) =>
  `${count} ${count === 1 ? 'learning item' : 'learning items'}`

const getLearningItemSummary = (item: LearningItem) => {
  const flashcard = item.variants.find((variant) => variant.format === 'flashcard')
  if (flashcard?.prompt.trim()) {
    return flashcard.prompt
  }

  const mcq = item.variants.find((variant) => variant.format === 'mcq')
  if (mcq?.prompt.trim()) {
    return mcq.prompt
  }

  return item.target?.trim() || `Learning item #${item.id}`
}

const toEditorValue = (item: LearningItem): LearningItemEditorValue => {
  const flashcard = item.variants.find((variant) => variant.format === 'flashcard')
  const mcq = item.variants.find((variant) => variant.format === 'mcq')
  const correctOption = mcq?.options.find((option) => option.id === mcq.correct_option_id)

  return {
    target: item.target ?? '',
    answer: item.answer ?? correctOption?.text ?? '',
    explanation: item.explanation,
    flashcard: flashcard ? { prompt: flashcard.prompt } : null,
    mcq: mcq
      ? {
          prompt: mcq.prompt,
          distractors: mcq.options
            .filter((option) => option.id !== mcq.correct_option_id)
            .map((option) => option.text),
        }
      : null,
    mcqOmissionReason: null,
  }
}

const toEditInput = (
  item: LearningItem,
  value: LearningItemEditorValue,
  spaceId: number,
): LearningItemEditInput => {
  const variants: LearningItemVariantInput[] = []
  const existingFlashcard = item.variants.find((variant) => variant.format === 'flashcard')
  const existingMcq = item.variants.find((variant) => variant.format === 'mcq')

  if (existingFlashcard && value.flashcard) {
    variants.push({
      format: 'flashcard',
      prompt: value.flashcard.prompt.trim(),
    })
  }

  if (existingMcq && value.mcq) {
    let distractorIndex = 0
    variants.push({
      format: 'mcq',
      prompt: value.mcq.prompt.trim(),
      correct_option_id: existingMcq.correct_option_id,
      options: existingMcq.options.map((option) => {
        if (option.id === existingMcq.correct_option_id) {
          return { ...option, text: value.answer.trim() }
        }

        const text = value.mcq?.distractors[distractorIndex]?.trim() ?? ''
        distractorIndex += 1
        return { ...option, text }
      }),
    })
  }

  const target = value.target.trim()
  const explanation = value.explanation?.trim() ?? ''

  return {
    target: target || null,
    answer: value.answer.trim(),
    explanation: explanation || null,
    space_id: spaceId,
    variants,
  }
}

export function QuestionsPage() {
  const navigate = useNavigate()
  const [spaces, setSpaces] = useState<RecallSpace[]>([])
  const [questions, setQuestions] = useState<LearningItem[]>([])
  const [allQuestions, setAllQuestions] = useState<LearningItem[]>([])
  const [spaceSummaries, setSpaceSummaries] = useState<RecallSpaceSummary[]>([])
  const [isLoadingSpaces, setIsLoadingSpaces] = useState(true)
  const [isLoadingQuestions, setIsLoadingQuestions] = useState(false)
  const [error, setError] = useState('')
  const [selectedSpace, setSelectedSpace] = useState<RecallSpace | null>(null)
  const [isManagingQuestions, setIsManagingQuestions] = useState(false)
  const [selectedQuestionIds, setSelectedQuestionIds] = useState<number[]>([])
  const [isDeletingQuestions, setIsDeletingQuestions] = useState(false)
  const [pendingDeleteQuestions, setPendingDeleteQuestions] = useState(false)
  const [editingSpace, setEditingSpace] = useState<RecallSpace | null>(null)
  const [editName, setEditName] = useState('')
  const [editDescription, setEditDescription] = useState('')
  const [isSavingSpace, setIsSavingSpace] = useState(false)
  const [editingItem, setEditingItem] = useState<LearningItem | null>(null)
  const [editValue, setEditValue] = useState<LearningItemEditorValue | null>(null)
  const [editSpaceId, setEditSpaceId] = useState(1)
  const [editItemError, setEditItemError] = useState('')
  const [isSavingItem, setIsSavingItem] = useState(false)
  const [isManagingSpaces, setIsManagingSpaces] = useState(false)
  const [deletingSpaceId, setDeletingSpaceId] = useState<number | null>(null)
  const [pendingDeleteSpaceId, setPendingDeleteSpaceId] = useState<number | null>(null)

  const pendingDeleteSpace =
    pendingDeleteSpaceId === null
      ? null
      : spaces.find((space) => space.id === pendingDeleteSpaceId) ?? null

  const refreshSpaceSummaries = async () => {
    try {
      const dashboard = await invoke<RecallDashboard>('get_recall_dashboard')
      setSpaceSummaries(dashboard.spaces)
    } catch {
      // A completed mutation remains valid even if its non-critical summary refresh fails.
    }
  }

  useEffect(() => {
    const loadSpacesAndCounts = async () => {
      setIsLoadingSpaces(true)
      setError('')

      try {
        const [spaceRows, itemRows, dashboard] = await Promise.all([
          invoke<RecallSpace[]>('get_spaces'),
          invoke<LearningItem[]>('get_learning_items', { spaceId: null }),
          invoke<RecallDashboard>('get_recall_dashboard'),
        ])
        setSpaces(spaceRows)
        setAllQuestions(itemRows)
        setSpaceSummaries(dashboard.spaces)
      } catch (err) {
        const message = err instanceof Error ? err.message : 'Failed to load recall spaces'
        setError(message)
      } finally {
        setIsLoadingSpaces(false)
      }
    }

    void loadSpacesAndCounts()
  }, [])

  const openSpace = async (space: RecallSpace) => {
    setSelectedSpace(space)
    setIsManagingQuestions(false)
    setSelectedQuestionIds([])
    setError('')
    setIsLoadingQuestions(true)

    try {
      const rows = await invoke<LearningItem[]>('get_learning_items', {
        spaceId: space.id,
      })
      setQuestions(rows)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load learning items'
      setError(message)
      setQuestions([])
    } finally {
      setIsLoadingQuestions(false)
    }
  }

  const backToSpaces = () => {
    setSelectedSpace(null)
    setQuestions([])
    setIsManagingQuestions(false)
    setSelectedQuestionIds([])
    setError('')
  }

  const toggleManageSpaces = () => {
    setIsManagingSpaces((current) => {
      const next = !current
      if (!next) {
        setPendingDeleteSpaceId(null)
      }
      return next
    })
  }

  const requestDeleteSpace = (space: RecallSpace) => {
    if (deletingSpaceId !== null || space.id === 1) {
      return
    }

    setPendingDeleteSpaceId(space.id)
    setError('')
  }

  const cancelDeleteSpace = () => {
    if (deletingSpaceId !== null) {
      return
    }

    setPendingDeleteSpaceId(null)
  }

  const confirmDeleteSpace = async (space: RecallSpace) => {
    if (deletingSpaceId !== null) {
      return
    }

    if (space.id === 1) {
      setError('General is the default space and cannot be deleted.')
      return
    }

    setError('')
    setDeletingSpaceId(space.id)

    try {
      await invoke('delete_space', { id: space.id })

      setSpaces((current) => current.filter((item) => item.id !== space.id))
      setAllQuestions((current) => current.filter((item) => item.space_id !== space.id))
      void refreshSpaceSummaries()
      setPendingDeleteSpaceId(null)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to delete recall space'
      setError(message)
    } finally {
      setDeletingSpaceId(null)
    }
  }

  const toggleManageQuestions = () => {
    setIsManagingQuestions((current) => {
      const next = !current
      if (!next) {
        setSelectedQuestionIds([])
      }
      return next
    })
  }

  const toggleSelectedQuestion = (id: number) => {
    setSelectedQuestionIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    )
  }

  const requestDeleteQuestions = () => {
    if (selectedQuestionIds.length === 0 || isDeletingQuestions) {
      return
    }
    setPendingDeleteQuestions(true)
  }

  const cancelDeleteQuestions = () => {
    if (isDeletingQuestions) {
      return
    }
    setPendingDeleteQuestions(false)
  }

  const deleteSelectedQuestions = async () => {
    if (selectedQuestionIds.length === 0 || isDeletingQuestions) {
      return
    }

    setError('')
    setIsDeletingQuestions(true)

    try {
      await invoke('delete_questions', {
        ids: selectedQuestionIds,
      })

      const selectedIdSet = new Set(selectedQuestionIds)

      setQuestions((current) => current.filter((item) => !selectedIdSet.has(item.id)))
      setAllQuestions((current) => current.filter((item) => !selectedIdSet.has(item.id)))
      void refreshSpaceSummaries()
      setSelectedQuestionIds([])
      setIsManagingQuestions(false)
      setPendingDeleteQuestions(false)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to delete selected questions'
      setError(message)
    } finally {
      setIsDeletingQuestions(false)
    }
  }

  const getSpaceQuestionCount = (spaceId: number) => {
    return allQuestions.filter((item) => item.space_id === spaceId).length
  }

  const getSpaceSummary = (spaceId: number) =>
    spaceSummaries.find((summary) => summary.id === spaceId) ?? {
      id: spaceId,
      total_questions: getSpaceQuestionCount(spaceId),
      due_count: 0,
      overdue_count: 0,
      new_count: 0,
    }

  const openEditSpace = (space: RecallSpace) => {
    setEditingSpace(space)
    setEditName(space.name)
    setEditDescription(space.description ?? '')
    setError('')
  }

  const cancelEditSpace = () => {
    if (isSavingSpace) return
    setEditingSpace(null)
  }

  const saveEditSpace = async () => {
    if (!editingSpace || isSavingSpace) return

    const trimmedName = editName.trim()
    if (trimmedName.length === 0) {
      setError('Space name cannot be empty.')
      return
    }

    setError('')
    setIsSavingSpace(true)

    try {
      const description = editDescription.trim().length > 0 ? editDescription.trim() : null

      const updated = await invoke<RecallSpace>('modify_space', {
        id: editingSpace.id,
        name: trimmedName,
        description,
      })

      setSpaces((current) => current.map((item) => (item.id === updated.id ? updated : item)))
      setEditingSpace(null)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to update recall space'
      setError(message)
    } finally {
      setIsSavingSpace(false)
    }
  }

  const openEditItem = (item: LearningItem) => {
    setEditingItem(item)
    setEditValue(toEditorValue(item))
    setEditSpaceId(item.space_id)
    setEditItemError('')
  }

  const cancelEditItem = () => {
    if (isSavingItem) return
    setEditingItem(null)
    setEditValue(null)
    setEditItemError('')
  }

  const saveEditItem = async () => {
    if (!editingItem || !editValue || isSavingItem) return

    const validationMessage = getLearningItemEditorError(editValue)
    if (validationMessage) {
      setEditItemError(validationMessage)
      return
    }

    setEditItemError('')
    setIsSavingItem(true)

    try {
      const updated = await invoke<LearningItem>('modify_learning_item', {
        id: editingItem.id,
        item: toEditInput(editingItem, editValue, editSpaceId),
      })

      setAllQuestions((current) =>
        current.map((item) => (item.id === updated.id ? updated : item)),
      )
      setQuestions((current) => {
        if (selectedSpace && updated.space_id !== selectedSpace.id) {
          return current.filter((item) => item.id !== updated.id)
        }
        return current.map((item) => (item.id === updated.id ? updated : item))
      })
      void refreshSpaceSummaries()
      setEditingItem(null)
      setEditValue(null)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to update learning item'
      setEditItemError(message)
    } finally {
      setIsSavingItem(false)
    }
  }

  const workload = spaceSummaries
    .filter((space) => !selectedSpace || space.id === selectedSpace.id)
    .reduce((total, space) => ({
      dueToday: total.dueToday + Math.max(0, space.due_count - space.overdue_count),
      overdue: total.overdue + space.overdue_count,
      newItems: total.newItems + space.new_count,
    }), { dueToday: 0, overdue: 0, newItems: 0 })

  return (
    <div className="app-container questions-page">
      <BackToHome />
      <header className="settings-panel">
        <h1>{selectedSpace ? selectedSpace.name : 'Learning Item Library'}</h1>
        <p className="settings-help-text">
          {selectedSpace
            ? 'Learning items saved inside this recall space.'
            : 'Browse, edit, and manage learning items by recall space.'}
        </p>
      </header>

      {error && <div className="error-banner">{error}</div>}

      {!isLoadingSpaces ? (
        <section className="library-workload-overview settings-panel" aria-labelledby="library-workload-heading">
          <div className="settings-section-head">
            <h2 id="library-workload-heading">Full workload</h2>
            <p>{selectedSpace ? 'All items in this Space.' : 'All items across your Spaces.'} Your daily plan selects a manageable amount from this workload.</p>
          </div>
          <RecallDonutChart label="Full workload breakdown"
            centerValue={workload.dueToday + workload.overdue + workload.newItems} centerLabel="Due + new items"
            categories={[
              { label: 'Due today', value: workload.dueToday, tone: 'due' },
              { label: 'Overdue', value: workload.overdue, tone: 'overdue' },
              { label: 'New', value: workload.newItems, tone: 'new' },
            ]} />
        </section>
      ) : null}

      {selectedSpace ? (
        <>
          <section className="questions-toolbar">
            <button type="button" className="btn-secondary btn-back" onClick={backToSpaces}>
              <ArrowLeft className="size-4" aria-hidden="true" />
              Back to spaces
            </button>

            <button
              type="button"
              className={`btn-secondary btn-manage-questions${isManagingQuestions ? ' is-active' : ''}`}
              onClick={toggleManageQuestions}
              aria-pressed={isManagingQuestions}
            >
              {isManagingQuestions ? 'Cancel Selection' : 'Manage learning items'}
            </button>
          </section>

          {isLoadingQuestions ? (
            <section className="settings-panel">
              <p className="settings-help-text">Loading learning items...</p>
            </section>
          ) : questions.length === 0 ? (
            <section className="settings-panel">
              <p className="settings-help-text">No learning items found for this space.</p>
            </section>
          ) : (
            <section className="questions-list" aria-label="Learning items list">
              {questions.map((item) => (
                <article className="question-card" key={item.id}>
                  <div className="question-card-head">
                    {isManagingQuestions && (
                      <label className="question-select" aria-label={`Select learning item ${item.id}`}>
                        <input
                          type="checkbox"
                          checked={selectedQuestionIds.includes(item.id)}
                          onChange={() => toggleSelectedQuestion(item.id)}
                        />
                      </label>
                    )}

                    <div className="question-toggle">
                      <div className="question-summary">
                        <span className="question-id">#{item.id}</span>
                        <h2>{getLearningItemSummary(item)}</h2>
                      </div>
                      <button
                        type="button"
                        className="question-more-btn"
                        onClick={() => openEditItem(item)}
                        aria-label={`Edit learning item ${item.id}`}
                        disabled={isManagingQuestions || isDeletingQuestions}
                      >
                        <Ellipsis className="size-4" aria-hidden="true" />
                      </button>
                    </div>
                  </div>
                </article>
              ))}
            </section>
          )}

          {isManagingQuestions && questions.length > 0 && (
            <section className="questions-manage-actions" aria-label="Learning item management actions">
              <button
                type="button"
                className="btn-primary btn-delete-questions"
                onClick={requestDeleteQuestions}
                disabled={selectedQuestionIds.length === 0 || isDeletingQuestions}
              >
                {selectedQuestionIds.length <= 1
                  ? 'Delete selected learning item'
                  : `Delete ${selectedQuestionIds.length} selected learning items`}
              </button>
            </section>
          )}

          {pendingDeleteQuestions && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelDeleteQuestions}
            >
              <div
                className="delete-space-modal"
                role="alertdialog"
                aria-modal="true"
                aria-label="Delete selected learning items"
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2>Delete learning items?</h2>
                <p>
                  <strong>
                    {selectedQuestionIds.length === 1
                      ? '1 learning item'
                      : `${selectedQuestionIds.length} learning items`}
                  </strong>{' '}
                  will be permanently deleted.
                </p>
                <div className="delete-space-modal-actions">
                  <button
                    type="button"
                    className="btn-secondary delete-space-cancel-btn"
                    onClick={cancelDeleteQuestions}
                    disabled={isDeletingQuestions}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn-primary delete-space-confirm-btn"
                    onClick={() => {
                      void deleteSelectedQuestions()
                    }}
                    disabled={isDeletingQuestions}
                  >
                    {isDeletingQuestions ? 'Deleting...' : 'Confirm delete'}
                  </button>
                </div>
              </div>
            </section>
          )}

          {editingItem && editValue && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelEditItem}
            >
              <div
                className="delete-space-modal edit-space-modal edit-learning-item-modal"
                role="dialog"
                aria-modal="true"
                aria-label={`Edit learning item ${editingItem.id}`}
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <header className="library-learning-item-header">
                  <div>
                    <h2>Edit learning item</h2>
                    <p>Update the shared knowledge and its saved practice variants.</p>
                  </div>
                  <span className={`library-learning-item-status is-${editingItem.recall_state}`}>
                    {editingItem.recall_state === 'new' ? 'New' : 'Scheduled'}
                  </span>
                </header>

                {(editingItem.generation.model ||
                  editingItem.generation.provider ||
                  editingItem.source) && (
                  <div className="library-learning-item-provenance" aria-label="Learning item provenance">
                    {editingItem.generation.model && (
                      <span title={`Model: ${editingItem.generation.model}`}>
                        Model <strong>{editingItem.generation.model}</strong>
                      </span>
                    )}
                    {editingItem.generation.provider && (
                      <span title={`Provider: ${editingItem.generation.provider}`}>
                        Provider <strong>{editingItem.generation.provider}</strong>
                      </span>
                    )}
                    {editingItem.source && (
                      <span
                        title={`${editingItem.source.note_path}:${editingItem.source.start_line}-${editingItem.source.end_line}`}
                      >
                        Source <strong>{editingItem.source.note_path}</strong>
                      </span>
                    )}
                  </div>
                )}

                <div className="library-learning-item-destination">
                  <label className="edit-space-label" htmlFor={`learning-item-space-${editingItem.id}`}>
                    Recall Space
                  </label>
                  <div className="recall-space-select-wrap edit-select-wrap">
                    <select
                      id={`learning-item-space-${editingItem.id}`}
                      className="recall-space-select edit-space-select"
                      value={editSpaceId}
                      onChange={(event) => setEditSpaceId(Number(event.target.value))}
                      disabled={isSavingItem}
                    >
                      {spaces.map((space) => (
                        <option key={space.id} value={space.id}>
                          {space.name}
                        </option>
                      ))}
                    </select>
                    <ChevronDown className="recall-space-chevron" aria-hidden="true" />
                  </div>
                </div>

                <LearningItemEditor
                  idPrefix={`library-${editingItem.id}`}
                  value={editValue}
                  onChange={(value) => {
                    setEditValue(value)
                    setEditItemError('')
                  }}
                  disabled={isSavingItem}
                />

                {editItemError && (
                  <p className="question-review-validation library-learning-item-validation" role="alert">
                    {editItemError}
                  </p>
                )}

                <div className="delete-space-modal-actions">
                  <button
                    type="button"
                    className="btn-secondary delete-space-cancel-btn"
                    onClick={cancelEditItem}
                    disabled={isSavingItem}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn-primary library-learning-item-save-btn"
                    onClick={() => {
                      void saveEditItem()
                    }}
                    disabled={isSavingItem}
                  >
                    {isSavingItem ? 'Saving...' : 'Save changes'}
                  </button>
                </div>
              </div>
            </section>
          )}
        </>
      ) : isLoadingSpaces ? (
        <section className="settings-panel">
          <p className="settings-help-text">Loading recall spaces...</p>
        </section>
      ) : spaces.length === 0 ? (
        <section className="settings-panel">
          <p className="settings-help-text">No recall spaces found.</p>
        </section>
      ) : (
        <>
          <section className="questions-toolbar">
            <div />
            <button
              type="button"
              className={`btn-secondary btn-manage-questions${isManagingSpaces ? ' is-active' : ''}`}
              onClick={toggleManageSpaces}
              aria-pressed={isManagingSpaces}
            >
              {isManagingSpaces ? 'Done' : 'Manage spaces'}
            </button>
          </section>

          <section className="recall-spaces-grid" aria-label="Recall spaces list">
            {spaces.map((space) => {
              const summary = getSpaceSummary(space.id)
              const isDefaultSpace = space.id === 1
              const isCaughtUp = summary.due_count === 0

              return (
                <article className={`recall-space-card${isManagingSpaces ? ' is-managing' : ''}`} key={space.id}>
                  <button
                    type="button"
                    className="recall-space-button"
                    onClick={() => {
                      if (deletingSpaceId !== null) {
                        return
                      }

                      void openSpace(space)
                    }}
                    disabled={deletingSpaceId !== null}
                  >
                    <div className="recall-space-meta">
                      <h2>{space.name}</h2>
                      <p>
                        {isCaughtUp ? 'No reviews due' : `${learningItemLabel(summary.due_count)} due`}
                        {' · '}{summary.new_count} new · {learningItemLabel(summary.total_questions)}
                      </p>
                    </div>
                  </button>

                  <div className="library-space-workload">
                    <span className={summary.overdue_count > 0 ? 'is-overdue' : undefined}>
                      {summary.overdue_count > 0
                        ? `${learningItemLabel(summary.overdue_count)} overdue`
                        : 'No overdue learning items'}
                    </span>
                    <div className="library-space-actions">
                      <button
                        type="button"
                        className="btn-secondary library-space-recall-btn"
                        onClick={() => {
                          navigate('/session', {
                            state: { recallSpaceId: space.id, recallSpaceName: space.name },
                          })
                        }}
                        disabled={(isCaughtUp && summary.new_count === 0) || deletingSpaceId !== null}
                      >
                        Study this Space
                        <ArrowRight className="size-4" aria-hidden="true" />
                      </button>
                    </div>
                  </div>

                  <button
                    type="button"
                    className="recall-space-more-btn"
                    onClick={(event) => {
                      event.stopPropagation()
                      openEditSpace(space)
                    }}
                    aria-label={`Edit ${space.name}`}
                    disabled={deletingSpaceId !== null}
                  >
                    <Ellipsis className="size-4" aria-hidden="true" />
                  </button>

                  {isManagingSpaces && (
                    <button
                      type="button"
                      className="recall-space-trash-btn"
                      onClick={() => requestDeleteSpace(space)}
                      aria-label={`Delete ${space.name}`}
                      title={isDefaultSpace ? 'General is the default space and cannot be deleted.' : undefined}
                      disabled={deletingSpaceId !== null || isDefaultSpace}
                    >
                      <Trash2 className="size-4" aria-hidden="true" />
                    </button>
                  )}
                </article>
              )
            })}
          </section>

          {isManagingSpaces && pendingDeleteSpace && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelDeleteSpace}
            >
              <div
                className="delete-space-modal"
                role="alertdialog"
                aria-modal="true"
                aria-label={`Delete ${pendingDeleteSpace.name}`}
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2>Delete recall space?</h2>
                <p>
                  <strong>{pendingDeleteSpace.name}</strong> and all learning items inside this space will be
                  permanently deleted.
                </p>
                <div className="delete-space-modal-actions">
                  <button
                    type="button"
                    className="btn-secondary delete-space-cancel-btn"
                    onClick={cancelDeleteSpace}
                    disabled={deletingSpaceId !== null}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn-primary delete-space-confirm-btn"
                    onClick={() => {
                      void confirmDeleteSpace(pendingDeleteSpace)
                    }}
                    disabled={deletingSpaceId !== null}
                  >
                    {deletingSpaceId !== null ? 'Deleting...' : 'Confirm delete'}
                  </button>
                </div>
              </div>
            </section>
          )}

          {editingSpace && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelEditSpace}
            >
              <div
                className="delete-space-modal edit-space-modal"
                role="dialog"
                aria-modal="true"
                aria-label={`Edit ${editingSpace.name}`}
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2>Edit recall space</h2>
                <div className="edit-space-form">
                  <label className="edit-space-label">
                    Title
                    <input
                      type="text"
                      className="edit-space-input"
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      disabled={isSavingSpace}
                      autoFocus
                    />
                  </label>
                  <label className="edit-space-label">
                    Description
                    <textarea
                      className="edit-space-input edit-space-textarea"
                      value={editDescription}
                      onChange={(e) => setEditDescription(e.target.value)}
                      disabled={isSavingSpace}
                      rows={3}
                    />
                  </label>
                </div>
                <div className="delete-space-modal-actions">
                  <button
                    type="button"
                    className="btn-secondary delete-space-cancel-btn"
                    onClick={cancelEditSpace}
                    disabled={isSavingSpace}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn-primary delete-space-confirm-btn"
                    onClick={() => {
                      void saveEditSpace()
                    }}
                    disabled={isSavingSpace}
                  >
                    {isSavingSpace ? 'Saving...' : 'Save'}
                  </button>
                </div>
              </div>
            </section>
          )}
        </>
      )}
    </div>
  )
}
