import { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowRight, ChevronDown, Ellipsis, Search, ShieldCheck, Sparkles, Trash2, X } from 'lucide-react'
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

type LibraryStatusFilter = 'all' | 'new' | 'scheduled' | 'overdue'
type LibraryFormatFilter = 'all' | 'mcq' | 'flashcard'

const statusFilters: { value: LibraryStatusFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'new', label: 'New' },
  { value: 'scheduled', label: 'Scheduled' },
  { value: 'overdue', label: 'Overdue' },
]

const formatFilters: { value: LibraryFormatFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'mcq', label: 'MCQ' },
  { value: 'flashcard', label: 'Flashcard' },
]

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

const isLearningItemOverdue = (item: LearningItem, todayUtc: string) =>
  item.recall_state === 'scheduled' &&
  Boolean(item.schedule.next_review_at) &&
  item.schedule.next_review_at!.slice(0, 10) < todayUtc

const getLearningItemSearchText = (item: LearningItem) => [
  item.id,
  item.target,
  item.answer,
  item.explanation,
  item.generation.model,
  item.generation.provider,
  item.generation.pipeline,
  item.source?.note_path,
  ...item.variants.flatMap((variant) => [
    variant.prompt,
    ...(variant.format === 'mcq' ? variant.options.map((option) => option.text) : []),
  ]),
]
  .filter((value): value is string | number => value !== null && value !== undefined)
  .join(' ')
  .normalize('NFKC')
  .toLocaleLowerCase()

const matchesStatusFilter = (
  item: LearningItem,
  filter: LibraryStatusFilter,
  todayUtc: string,
) => {
  if (filter === 'all') return true
  if (filter === 'overdue') return isLearningItemOverdue(item, todayUtc)
  if (filter === 'scheduled') {
    return item.recall_state === 'scheduled' && !isLearningItemOverdue(item, todayUtc)
  }
  return item.recall_state === filter
}

const matchesFormatFilter = (item: LearningItem, filter: LibraryFormatFilter) =>
  filter === 'all' || item.variants.some((variant) => variant.format === filter)

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

const getModalFocusableElements = (dialog: HTMLElement) =>
  Array.from(
    dialog.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((element) => !element.hasAttribute('hidden'))

const useModalFocus = (
  isOpen: boolean,
  isBusy: boolean,
  onClose: () => void,
  fallbackFocusSelector: string,
) => {
  const dialogRef = useRef<HTMLDivElement>(null)
  const previousFocusRef = useRef<HTMLElement | null>(null)
  const isBusyRef = useRef(isBusy)
  const onCloseRef = useRef(onClose)

  useEffect(() => {
    isBusyRef.current = isBusy
    onCloseRef.current = onClose
  }, [isBusy, onClose])

  useEffect(() => {
    if (!isOpen) return

    const dialog = dialogRef.current
    previousFocusRef.current = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null

    const focusFrame = window.requestAnimationFrame(() => {
      const firstFocusable = dialog ? getModalFocusableElements(dialog)[0] : null
      const focusTarget = firstFocusable ?? dialog
      focusTarget?.focus()
    })

    const handleKeyDown = (event: KeyboardEvent) => {
      if (!dialog) return

      if (event.key === 'Escape' && !isBusyRef.current) {
        event.preventDefault()
        onCloseRef.current()
        return
      }

      if (event.key !== 'Tab') return

      const focusableElements = getModalFocusableElements(dialog)
      if (focusableElements.length === 0) {
        event.preventDefault()
        dialog.focus()
        return
      }

      const firstFocusable = focusableElements[0]
      const lastFocusable = focusableElements[focusableElements.length - 1]
      const activeElement = document.activeElement

      if (event.shiftKey && (activeElement === firstFocusable || !dialog.contains(activeElement))) {
        event.preventDefault()
        lastFocusable.focus()
      } else if (!event.shiftKey && activeElement === lastFocusable) {
        event.preventDefault()
        firstFocusable.focus()
      }
    }

    document.addEventListener('keydown', handleKeyDown)

    return () => {
      window.cancelAnimationFrame(focusFrame)
      document.removeEventListener('keydown', handleKeyDown)
      const previousFocus = previousFocusRef.current
      const fallbackFocus = document.querySelector<HTMLElement>(fallbackFocusSelector)
      window.requestAnimationFrame(() => {
        if (previousFocus?.isConnected) {
          previousFocus.focus()
        } else {
          fallbackFocus?.focus()
        }
      })
    }
  }, [fallbackFocusSelector, isOpen])

  return dialogRef
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
  const [selectedSpaceIds, setSelectedSpaceIds] = useState<number[]>([])
  const [isDeletingSpaces, setIsDeletingSpaces] = useState(false)
  const [pendingDeleteSpaces, setPendingDeleteSpaces] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [statusFilter, setStatusFilter] = useState<LibraryStatusFilter>('all')
  const [formatFilter, setFormatFilter] = useState<LibraryFormatFilter>('all')
  const selectAllSpacesRef = useRef<HTMLInputElement>(null)
  const selectAllQuestionsRef = useRef<HTMLInputElement>(null)

  const normalizedSearchQuery = searchQuery.trim().normalize('NFKC').toLocaleLowerCase()
  const todayUtc = new Date().toISOString().slice(0, 10)
  const filteredQuestions = useMemo(
    () => questions.filter((item) =>
      matchesStatusFilter(item, statusFilter, todayUtc) &&
      matchesFormatFilter(item, formatFilter) &&
      (!normalizedSearchQuery || getLearningItemSearchText(item).includes(normalizedSearchQuery)),
    ),
    [formatFilter, normalizedSearchQuery, questions, statusFilter, todayUtc],
  )
  const hasActiveLibraryFilters = Boolean(normalizedSearchQuery) || statusFilter !== 'all' || formatFilter !== 'all'

  const deletableSpaces = spaces.filter((space) => space.id !== 1)
  const areAllSpacesSelected =
    deletableSpaces.length > 0 && selectedSpaceIds.length === deletableSpaces.length
  const areSomeSpacesSelected = selectedSpaceIds.length > 0 && !areAllSpacesSelected
  const areAllQuestionsSelected =
    filteredQuestions.length > 0 && selectedQuestionIds.length === filteredQuestions.length
  const areSomeQuestionsSelected =
    selectedQuestionIds.length > 0 && !areAllQuestionsSelected
  const selectedSpaceIdSet = new Set(selectedSpaceIds)
  const selectedSpaceLearningItemCount = allQuestions.filter((item) =>
    selectedSpaceIdSet.has(item.space_id),
  ).length

  useEffect(() => {
    if (selectAllSpacesRef.current) {
      selectAllSpacesRef.current.indeterminate = areSomeSpacesSelected
    }
  }, [areSomeSpacesSelected])

  useEffect(() => {
    if (selectAllQuestionsRef.current) {
      selectAllQuestionsRef.current.indeterminate = areSomeQuestionsSelected
    }
  }, [areSomeQuestionsSelected])

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
    setSearchQuery('')
    setStatusFilter('all')
    setFormatFilter('all')
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
    setSearchQuery('')
    setStatusFilter('all')
    setFormatFilter('all')
    setError('')
  }

  const toggleManageSpaces = () => {
    setIsManagingSpaces((current) => {
      const next = !current
      if (!next) {
        setSelectedSpaceIds([])
        setPendingDeleteSpaces(false)
      }
      return next
    })
  }

  const toggleSelectedSpace = (id: number) => {
    if (id === 1) return
    setSelectedSpaceIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    )
  }

  const toggleAllSpaces = () => {
    setSelectedSpaceIds(
      areAllSpacesSelected ? [] : deletableSpaces.map((space) => space.id),
    )
  }

  const requestDeleteSpaces = () => {
    if (selectedSpaceIds.length === 0 || isDeletingSpaces) return
    setPendingDeleteSpaces(true)
    setError('')
  }

  const cancelDeleteSpaces = () => {
    if (isDeletingSpaces) return
    setPendingDeleteSpaces(false)
  }

  const confirmDeleteSpaces = async () => {
    if (selectedSpaceIds.length === 0 || isDeletingSpaces) return
    setError('')
    setIsDeletingSpaces(true)

    try {
      const idsToDelete = [...selectedSpaceIds]
      const results = await Promise.allSettled(
        idsToDelete.map((id) => invoke('delete_space', { id })),
      )
      const deletedIds = idsToDelete.filter((_, index) => results[index].status === 'fulfilled')
      const failedIds = idsToDelete.filter((_, index) => results[index].status === 'rejected')
      const deletedIdSet = new Set(deletedIds)

      setSpaces((current) => current.filter((item) => !deletedIdSet.has(item.id)))
      setAllQuestions((current) => current.filter((item) => !deletedIdSet.has(item.space_id)))
      void refreshSpaceSummaries()
      setPendingDeleteSpaces(false)

      if (failedIds.length > 0) {
        setSelectedSpaceIds(failedIds)
        setError(
          `${deletedIds.length} ${deletedIds.length === 1 ? 'Space was' : 'Spaces were'} deleted, but ${failedIds.length} could not be deleted. Try again.`,
        )
      } else {
        setSelectedSpaceIds([])
        setIsManagingSpaces(false)
      }
    } finally {
      setIsDeletingSpaces(false)
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

  const toggleAllQuestions = () => {
    setSelectedQuestionIds(
      areAllQuestionsSelected ? [] : filteredQuestions.map((question) => question.id),
    )
  }

  const updateSearchQuery = (value: string) => {
    setSearchQuery(value)
    setSelectedQuestionIds([])
  }

  const updateStatusFilter = (value: LibraryStatusFilter) => {
    setStatusFilter(value)
    setSelectedQuestionIds([])
  }

  const updateFormatFilter = (value: LibraryFormatFilter) => {
    setFormatFilter(value)
    setSelectedQuestionIds([])
  }

  const clearLibraryFilters = () => {
    setSearchQuery('')
    setStatusFilter('all')
    setFormatFilter('all')
    setSelectedQuestionIds([])
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

  const deleteQuestionsDialogRef = useModalFocus(
    pendingDeleteQuestions,
    isDeletingQuestions,
    cancelDeleteQuestions,
    '.btn-manage-questions',
  )
  const deleteSpacesDialogRef = useModalFocus(
    pendingDeleteSpaces,
    isDeletingSpaces,
    cancelDeleteSpaces,
    '.btn-manage-questions',
  )

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
              {isManagingQuestions ? 'Cancel selection' : 'Select items'}
            </button>
          </section>

          {!isLoadingQuestions && questions.length > 0 && (
            <section className="library-filter-panel" aria-label={`Search and filter ${selectedSpace.name}`}>
              <div className="library-search-row">
                <label className="library-search-field" htmlFor="library-space-search">
                  <span className="library-filter-label">Search this Space</span>
                  <span className="library-search-control">
                    <Search aria-hidden="true" />
                    <input
                      id="library-space-search"
                      type="search"
                      value={searchQuery}
                      onChange={(event) => updateSearchQuery(event.target.value)}
                      placeholder="Search prompts, answers, or source notes"
                      autoComplete="off"
                    />
                    {searchQuery ? (
                      <button
                        type="button"
                        className="library-search-clear"
                        onClick={() => updateSearchQuery('')}
                        aria-label="Clear search"
                      >
                        <X aria-hidden="true" />
                      </button>
                    ) : null}
                  </span>
                </label>

                <div className="library-result-summary" role="status" aria-live="polite">
                  <strong>{filteredQuestions.length}</strong>
                  <span>
                    {hasActiveLibraryFilters
                      ? `of ${learningItemLabel(questions.length)}`
                      : filteredQuestions.length === 1 ? 'learning item' : 'learning items'}
                  </span>
                </div>
              </div>

              <div className="library-filter-row">
                <div className="library-filter-group" role="group" aria-label="Filter by review status">
                  <span className="library-filter-label">Status</span>
                  <div className="library-filter-options">
                    {statusFilters.map((filter) => (
                      <button
                        key={filter.value}
                        type="button"
                        className={`library-filter-chip${statusFilter === filter.value ? ' is-active' : ''}`}
                        aria-pressed={statusFilter === filter.value}
                        onClick={() => updateStatusFilter(filter.value)}
                      >
                        {filter.label}
                      </button>
                    ))}
                  </div>
                </div>

                <div className="library-filter-group" role="group" aria-label="Filter by practice format">
                  <span className="library-filter-label">Format</span>
                  <div className="library-filter-options">
                    {formatFilters.map((filter) => (
                      <button
                        key={filter.value}
                        type="button"
                        className={`library-filter-chip${formatFilter === filter.value ? ' is-active' : ''}`}
                        aria-pressed={formatFilter === filter.value}
                        onClick={() => updateFormatFilter(filter.value)}
                      >
                        {filter.label}
                      </button>
                    ))}
                  </div>
                </div>

                {hasActiveLibraryFilters ? (
                  <button type="button" className="library-clear-filters" onClick={clearLibraryFilters}>
                    Clear filters
                  </button>
                ) : null}
              </div>
            </section>
          )}

          {isManagingQuestions && !isLoadingQuestions && filteredQuestions.length > 0 && (
            <section className="bulk-selection-bar" aria-label="Bulk selection controls">
              <div className="bulk-selection-summary">
                <label className="bulk-select-all">
                  <input
                    ref={selectAllQuestionsRef}
                    type="checkbox"
                    checked={areAllQuestionsSelected}
                    onChange={toggleAllQuestions}
                  />
                  <span>
                    Select all {filteredQuestions.length}
                    {hasActiveLibraryFilters ? ' results' : ''}
                  </span>
                </label>
                <span className="bulk-selected-count" role="status" aria-live="polite">
                  {selectedQuestionIds.length} selected
                </span>
              </div>
              <button
                type="button"
                className="btn-primary btn-delete-selection"
                onClick={requestDeleteQuestions}
                disabled={selectedQuestionIds.length === 0 || isDeletingQuestions}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                {selectedQuestionIds.length === 0
                  ? 'Delete selected'
                  : selectedQuestionIds.length === 1
                    ? 'Delete 1 item'
                    : `Delete ${selectedQuestionIds.length} items`}
              </button>
            </section>
          )}

          {isLoadingQuestions ? (
            <section className="settings-panel">
              <p className="settings-help-text">Loading learning items...</p>
            </section>
          ) : questions.length === 0 ? (
            <section className="library-empty-state" aria-labelledby="empty-space-heading">
              <Sparkles aria-hidden="true" />
              <div>
                <h2 id="empty-space-heading">No learning items in this Space</h2>
                <p>Generate learning items from your notes, then save them to {selectedSpace.name}.</p>
              </div>
              <div className="library-empty-actions">
                <button type="button" className="btn-primary" onClick={() => navigate('/')}>
                  Generate items<ArrowRight className="size-4" aria-hidden="true" />
                </button>
                <button type="button" className="btn-secondary" onClick={backToSpaces}>
                  Back to Spaces
                </button>
              </div>
            </section>
          ) : filteredQuestions.length === 0 ? (
            <section className="library-no-results" aria-live="polite">
              <Search aria-hidden="true" />
              <div>
                <h2>No matching learning items</h2>
                <p>Try a different search or clear the filters to see everything in this Space.</p>
              </div>
              <button type="button" className="btn-secondary" onClick={clearLibraryFilters}>
                Clear filters
              </button>
            </section>
          ) : (
            <section className="questions-list" aria-label="Learning items list">
              {filteredQuestions.map((item) => {
                const isOverdue = isLearningItemOverdue(item, todayUtc)
                const statusLabel = isOverdue
                  ? 'Overdue'
                  : item.recall_state === 'new'
                    ? 'New'
                    : 'Scheduled'

                return (
                <article
                  className={`question-card${isManagingQuestions ? ' is-selectable' : ''}${selectedQuestionIds.includes(item.id) ? ' is-selected' : ''}`}
                  key={item.id}
                  onClick={isManagingQuestions ? () => toggleSelectedQuestion(item.id) : undefined}
                >
                  <div className="question-card-head">
                    {isManagingQuestions && (
                      <label className="question-select" aria-label={`Select learning item ${item.id}`}>
                        <input
                          type="checkbox"
                          checked={selectedQuestionIds.includes(item.id)}
                          onChange={() => toggleSelectedQuestion(item.id)}
                          onClick={(event) => event.stopPropagation()}
                        />
                      </label>
                    )}

                    <div className="question-toggle">
                      <div className="question-summary">
                        <div className="question-summary-meta">
                          <span className="question-id">#{item.id}</span>
                          <span className={`question-state-badge is-${statusLabel.toLocaleLowerCase()}`}>
                            {statusLabel}
                          </span>
                          {item.variants.map((variant) => (
                            <span className="question-format-badge" key={variant.id}>
                              {variant.format === 'mcq' ? 'MCQ' : 'Flashcard'}
                            </span>
                          ))}
                        </div>
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
                )
              })}
            </section>
          )}

          {pendingDeleteQuestions && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelDeleteQuestions}
            >
              <div
                ref={deleteQuestionsDialogRef}
                className="delete-space-modal"
                role="alertdialog"
                aria-modal="true"
                aria-labelledby="delete-learning-items-heading"
                aria-describedby="delete-learning-items-description"
                tabIndex={-1}
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2 id="delete-learning-items-heading">Delete learning items?</h2>
                <p id="delete-learning-items-description">
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
        <section className="library-empty-state" aria-labelledby="empty-library-heading">
          <Sparkles aria-hidden="true" />
          <div>
            <h2 id="empty-library-heading">No Recall Spaces yet</h2>
            <p>Generate learning items from a Markdown note to create your first Space.</p>
          </div>
          <div className="library-empty-actions">
            <button type="button" className="btn-primary" onClick={() => navigate('/')}>
              Generate from notes<ArrowRight className="size-4" aria-hidden="true" />
            </button>
          </div>
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
              disabled={deletableSpaces.length === 0}
              title={deletableSpaces.length === 0 ? 'General is the only Space and cannot be deleted.' : undefined}
            >
              {isManagingSpaces ? 'Cancel selection' : 'Select Spaces'}
            </button>
          </section>

          {isManagingSpaces && deletableSpaces.length > 0 && (
            <section className="bulk-selection-bar" aria-label="Bulk Space selection controls">
              <div className="bulk-selection-summary">
                <label className="bulk-select-all">
                  <input
                    ref={selectAllSpacesRef}
                    type="checkbox"
                    checked={areAllSpacesSelected}
                    onChange={toggleAllSpaces}
                  />
                  <span>Select all {deletableSpaces.length}</span>
                </label>
                <span className="bulk-selected-count" role="status" aria-live="polite">
                  {selectedSpaceIds.length} selected
                </span>
              </div>
              <button
                type="button"
                className="btn-primary btn-delete-selection"
                onClick={requestDeleteSpaces}
                disabled={selectedSpaceIds.length === 0 || isDeletingSpaces}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                {selectedSpaceIds.length === 0
                  ? 'Delete selected'
                  : selectedSpaceIds.length === 1
                    ? 'Delete 1 Space'
                    : `Delete ${selectedSpaceIds.length} Spaces`}
              </button>
            </section>
          )}

          <section className="recall-spaces-grid" aria-label="Recall spaces list">
            {spaces.map((space) => {
              const summary = getSpaceSummary(space.id)
              const isDefaultSpace = space.id === 1
              const isSelected = selectedSpaceIds.includes(space.id)
              const isCaughtUp = summary.due_count === 0

              return (
                <article
                  className={`recall-space-card${isManagingSpaces ? ' is-managing' : ''}${isSelected ? ' is-selected' : ''}${isDefaultSpace ? ' is-default' : ''}`}
                  key={space.id}
                >
                  <button
                    type="button"
                    className="recall-space-button"
                    onClick={() => {
                      if (isManagingSpaces || isDeletingSpaces) {
                        return
                      }

                      void openSpace(space)
                    }}
                    disabled={isManagingSpaces || isDeletingSpaces}
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
                        disabled={(isCaughtUp && summary.new_count === 0) || isManagingSpaces || isDeletingSpaces}
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
                    disabled={isManagingSpaces || isDeletingSpaces}
                  >
                    <Ellipsis className="size-4" aria-hidden="true" />
                  </button>

                  {isManagingSpaces && (isDefaultSpace ? (
                    <div
                      className="recall-space-selection-protected"
                      title="General is the default Space and cannot be deleted."
                    >
                      <span><ShieldCheck className="size-4" aria-hidden="true" /> Default</span>
                    </div>
                  ) : (
                    <label className="recall-space-selection-target">
                      <input
                        type="checkbox"
                        checked={isSelected}
                        onChange={() => toggleSelectedSpace(space.id)}
                        aria-label={`Select ${space.name}`}
                      />
                    </label>
                  ))}
                </article>
              )
            })}
          </section>

          {isManagingSpaces && pendingDeleteSpaces && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelDeleteSpaces}
            >
              <div
                ref={deleteSpacesDialogRef}
                className="delete-space-modal"
                role="alertdialog"
                aria-modal="true"
                aria-labelledby="delete-spaces-heading"
                aria-describedby="delete-spaces-description"
                tabIndex={-1}
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2 id="delete-spaces-heading">
                  {selectedSpaceIds.length === 1 ? 'Delete selected Space?' : 'Delete selected Spaces?'}
                </h2>
                <p id="delete-spaces-description">
                  <strong>
                    {selectedSpaceIds.length === 1 ? '1 Space' : `${selectedSpaceIds.length} Spaces`}
                  </strong>{' '}
                  and{' '}
                  <strong>
                    {selectedSpaceLearningItemCount === 1
                      ? '1 learning item'
                      : `${selectedSpaceLearningItemCount} learning items`}
                  </strong>{' '}
                  {selectedSpaceIds.length === 1 ? 'inside it' : 'inside them'} will be permanently deleted. General will remain untouched.
                </p>
                <div className="delete-space-modal-actions">
                  <button
                    type="button"
                    className="btn-secondary delete-space-cancel-btn"
                    onClick={cancelDeleteSpaces}
                    disabled={isDeletingSpaces}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn-primary delete-space-confirm-btn"
                    onClick={() => {
                      void confirmDeleteSpaces()
                    }}
                    disabled={isDeletingSpaces}
                  >
                    {isDeletingSpaces
                      ? 'Deleting...'
                      : selectedSpaceIds.length === 1
                        ? 'Delete 1 Space'
                        : `Delete ${selectedSpaceIds.length} Spaces`}
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
