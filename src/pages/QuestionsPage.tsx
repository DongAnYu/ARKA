import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ChevronDown, Ellipsis, Trash2 } from 'lucide-react'

type Question = {
  id: number
  question: string
  option_a: string
  option_b: string
  option_c: string
  option_d: string
  correct_answer: string
  explanation: string | null
  model: string | null
  space_id: number
}

type RecallSpace = {
  id: number
  name: string
  description: string | null
}

export function QuestionsPage() {
  const [spaces, setSpaces] = useState<RecallSpace[]>([])
  const [questions, setQuestions] = useState<Question[]>([])
  const [allQuestions, setAllQuestions] = useState<Question[]>([])
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
  const [editingQuestion, setEditingQuestion] = useState<Question | null>(null)
  const [editQuestionText, setEditQuestionText] = useState('')
  const [editOptionA, setEditOptionA] = useState('')
  const [editOptionB, setEditOptionB] = useState('')
  const [editOptionC, setEditOptionC] = useState('')
  const [editOptionD, setEditOptionD] = useState('')
  const [editCorrectAnswer, setEditCorrectAnswer] = useState<'A' | 'B' | 'C' | 'D'>('A')
  const [isSavingQuestion, setIsSavingQuestion] = useState(false)
  const [isManagingSpaces, setIsManagingSpaces] = useState(false)
  const [deletingSpaceId, setDeletingSpaceId] = useState<number | null>(null)
  const [pendingDeleteSpaceId, setPendingDeleteSpaceId] = useState<number | null>(null)

  const pendingDeleteSpace =
    pendingDeleteSpaceId === null
      ? null
      : spaces.find((space) => space.id === pendingDeleteSpaceId) ?? null

  useEffect(() => {
    const loadSpacesAndCounts = async () => {
      setIsLoadingSpaces(true)
      setError('')

      try {
        const [spaceRows, questionRows] = await Promise.all([
          invoke<RecallSpace[]>('get_spaces'),
          invoke<Question[]>('get_questions'),
        ])
        setSpaces(spaceRows)
        setAllQuestions(questionRows)
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
      const rows = await invoke<Question[]>('get_questions_by_space', {
        spaceId: space.id,
      })
      setQuestions(rows)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load questions'
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

  const openEditQuestion = (question: Question) => {
    setEditingQuestion(question)
    setEditQuestionText(question.question)
    setEditOptionA(question.option_a)
    setEditOptionB(question.option_b)
    setEditOptionC(question.option_c)
    setEditOptionD(question.option_d)
    const normalizedAnswer = question.correct_answer.trim().toUpperCase()
    if (
      normalizedAnswer === 'A' ||
      normalizedAnswer === 'B' ||
      normalizedAnswer === 'C' ||
      normalizedAnswer === 'D'
    ) {
      setEditCorrectAnswer(normalizedAnswer)
    } else {
      setEditCorrectAnswer('A')
    }
    setError('')
  }

  const cancelEditQuestion = () => {
    if (isSavingQuestion) return
    setEditingQuestion(null)
  }

  const saveEditQuestion = async () => {
    if (!editingQuestion || isSavingQuestion) return

    const trimmedQuestion = editQuestionText.trim()
    if (trimmedQuestion.length === 0) {
      setError('Question text cannot be empty.')
      return
    }

    if (
      editCorrectAnswer !== 'A' &&
      editCorrectAnswer !== 'B' &&
      editCorrectAnswer !== 'C' &&
      editCorrectAnswer !== 'D'
    ) {
      setError('Correct answer must be one of A, B, C, or D.')
      return
    }

    setError('')
    setIsSavingQuestion(true)

    try {
      const updated = await invoke<Question>('modify_question', {
        id: editingQuestion.id,
        questionInput: {
          question: trimmedQuestion,
          option_a: editOptionA.trim(),
          option_b: editOptionB.trim(),
          option_c: editOptionC.trim(),
          option_d: editOptionD.trim(),
          correct_answer: editCorrectAnswer,
          explanation: editingQuestion.explanation,
          space_id: editingQuestion.space_id,
        },
      })

      setQuestions((current) => current.map((item) => (item.id === updated.id ? updated : item)))
      setAllQuestions((current) => current.map((item) => (item.id === updated.id ? updated : item)))
      setEditingQuestion(null)
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to update question'
      setError(message)
    } finally {
      setIsSavingQuestion(false)
    }
  }

  return (
    <div className="app-container questions-page">
      <header className="settings-panel">
        <h1>{selectedSpace ? selectedSpace.name : 'Recall Spaces'}</h1>
        <p className="settings-help-text">
          {selectedSpace
            ? 'Questions saved inside this recall space.'
            : 'Choose a recall space to browse its questions.'}
        </p>
      </header>

      {error && <div className="error-banner">{error}</div>}

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
              {isManagingQuestions ? 'Cancel Selection' : 'Manage questions'}
            </button>
          </section>

          {isLoadingQuestions ? (
            <section className="settings-panel">
              <p className="settings-help-text">Loading questions...</p>
            </section>
          ) : questions.length === 0 ? (
            <section className="settings-panel">
              <p className="settings-help-text">No questions found for this space.</p>
            </section>
          ) : (
            <section className="questions-list" aria-label="Questions list">
              {questions.map((item) => (
                <article className="question-card" key={item.id}>
                  <div className="question-card-head">
                    {isManagingQuestions && (
                      <label className="question-select" aria-label={`Select question ${item.id}`}>
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
                        <h2>{item.question}</h2>
                      </div>
                      <button
                        type="button"
                        className="question-more-btn"
                        onClick={() => openEditQuestion(item)}
                        aria-label={`Edit question ${item.id}`}
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
            <section className="questions-manage-actions" aria-label="Question management actions">
              <button
                type="button"
                className="btn-primary btn-delete-questions"
                onClick={requestDeleteQuestions}
                disabled={selectedQuestionIds.length === 0 || isDeletingQuestions}
              >
                {selectedQuestionIds.length <= 1
                  ? 'Delete selected question'
                  : `Delete ${selectedQuestionIds.length} selected questions`}
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
                aria-label="Delete selected questions"
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2>Delete questions?</h2>
                <p>
                  <strong>
                    {selectedQuestionIds.length === 1
                      ? '1 question'
                      : `${selectedQuestionIds.length} questions`}
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

          {editingQuestion && (
            <section
              className="delete-space-modal-overlay"
              role="presentation"
              onClick={cancelEditQuestion}
            >
              <div
                className="delete-space-modal edit-space-modal edit-question-modal"
                role="dialog"
                aria-modal="true"
                aria-label={`Edit question ${editingQuestion.id}`}
                onClick={(event) => {
                  event.stopPropagation()
                }}
              >
                <h2>Edit question</h2>
                <div className="edit-space-form">
                  <label className="edit-space-label">
                    Question
                    <textarea
                      className="edit-space-input edit-space-textarea"
                      value={editQuestionText}
                      onChange={(e) => setEditQuestionText(e.target.value)}
                      disabled={isSavingQuestion}
                      rows={2}
                      autoFocus
                    />
                  </label>
                  <div className="edit-question-options-grid">
                    <label className="edit-space-label">
                      Option A
                      <textarea
                        className="edit-space-input edit-question-option-textarea"
                        value={editOptionA}
                        onChange={(e) => setEditOptionA(e.target.value)}
                        disabled={isSavingQuestion}
                        rows={2}
                      />
                    </label>
                    <label className="edit-space-label">
                      Option B
                      <textarea
                        className="edit-space-input edit-question-option-textarea"
                        value={editOptionB}
                        onChange={(e) => setEditOptionB(e.target.value)}
                        disabled={isSavingQuestion}
                        rows={2}
                      />
                    </label>
                    <label className="edit-space-label">
                      Option C
                      <textarea
                        className="edit-space-input edit-question-option-textarea"
                        value={editOptionC}
                        onChange={(e) => setEditOptionC(e.target.value)}
                        disabled={isSavingQuestion}
                        rows={2}
                      />
                    </label>
                    <label className="edit-space-label">
                      Option D
                      <textarea
                        className="edit-space-input edit-question-option-textarea"
                        value={editOptionD}
                        onChange={(e) => setEditOptionD(e.target.value)}
                        disabled={isSavingQuestion}
                        rows={2}
                      />
                    </label>
                  </div>
                  <label className="edit-space-label">
                    Correct Answer
                    <div className="recall-space-select-wrap edit-select-wrap">
                      <select
                        className="recall-space-select edit-space-select"
                        value={editCorrectAnswer}
                        onChange={(event) =>
                          setEditCorrectAnswer(event.target.value as 'A' | 'B' | 'C' | 'D')
                        }
                        disabled={isSavingQuestion}
                      >
                        <option value="A">A</option>
                        <option value="B">B</option>
                        <option value="C">C</option>
                        <option value="D">D</option>
                      </select>
                      <ChevronDown className="recall-space-chevron" aria-hidden="true" />
                    </div>
                  </label>
                </div>
                <div className="delete-space-modal-actions">
                  <button
                    type="button"
                    className="btn-secondary delete-space-cancel-btn"
                    onClick={cancelEditQuestion}
                    disabled={isSavingQuestion}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="btn-primary delete-space-confirm-btn"
                    onClick={() => {
                      void saveEditQuestion()
                    }}
                    disabled={isSavingQuestion}
                  >
                    {isSavingQuestion ? 'Saving...' : 'Save'}
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
              const questionCount = getSpaceQuestionCount(space.id)
              const isDefaultSpace = space.id === 1

              return (
                <article className={`recall-space-card${isManagingSpaces ? ' is-managing' : ''}`} key={space.id}>
                  <div
                    className="recall-space-button"
                    onClick={() => {
                      if (deletingSpaceId !== null) {
                        return
                      }

                      void openSpace(space)
                    }}
                    onKeyDown={(event) => {
                      if (deletingSpaceId !== null) {
                        return
                      }

                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault()
                        void openSpace(space)
                      }
                    }}
                    aria-label={`Open ${space.name}`}
                    role="button"
                    tabIndex={deletingSpaceId !== null ? -1 : 0}
                    aria-disabled={deletingSpaceId !== null}
                  >
                    <div className="recall-space-meta">
                      <span className="question-id">Space #{space.id}</span>
                      <h2>{space.name}</h2>
                      <p>
                        {space.description?.trim() || 'No description provided.'}
                      </p>
                    </div>

                    <div className="recall-space-count" aria-label={`${questionCount} questions`}>
                      <strong>{questionCount}</strong>
                      <span>{questionCount === 1 ? 'Question' : 'Questions'}</span>
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
                  <strong>{pendingDeleteSpace.name}</strong> and all questions inside this space will be
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
