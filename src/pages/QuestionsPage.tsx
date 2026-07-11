import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ChevronDown, Trash2 } from 'lucide-react'
import { EditableText } from '../components/EditableText'

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
  const [expandedIds, setExpandedIds] = useState<number[]>([])
  const [selectedSpace, setSelectedSpace] = useState<RecallSpace | null>(null)
  const [isManagingQuestions, setIsManagingQuestions] = useState(false)
  const [selectedQuestionIds, setSelectedQuestionIds] = useState<number[]>([])
  const [isDeletingQuestions, setIsDeletingQuestions] = useState(false)
  const [isManagingSpaces, setIsManagingSpaces] = useState(false)
  const [deletingSpaceId, setDeletingSpaceId] = useState<number | null>(null)
  const [pendingDeleteSpaceId, setPendingDeleteSpaceId] = useState<number | null>(null)

  const pendingDeleteSpace =
    pendingDeleteSpaceId === null
      ? null
      : spaces.find((space) => space.id === pendingDeleteSpaceId) ?? null

  const toggleExpanded = (id: number) => {
    setExpandedIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    )
  }

  const isCorrectOption = (choice: string, label: 'A' | 'B' | 'C' | 'D', q: Question) => {
    const answer = q.correct_answer.trim().toLowerCase()
    const normalizedChoice = choice.trim().toLowerCase()
    const normalizedLabel = label.toLowerCase()

    return (
      answer === normalizedLabel ||
      answer === `option_${normalizedLabel}` ||
      answer === normalizedChoice
    )
  }

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
    setExpandedIds([])
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
    setExpandedIds([])
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
      setExpandedIds((current) => current.filter((id) => !selectedIdSet.has(id)))
      setSelectedQuestionIds([])
      setIsManagingQuestions(false)
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

  const saveSpaceField = async (space: RecallSpace, field: 'name' | 'description', nextValue: string) => {
    const name = field === 'name' ? nextValue : space.name
    const description = field === 'description' ? (nextValue.length > 0 ? nextValue : null) : space.description

    const updated = await invoke<RecallSpace>('modify_space', {
      id: space.id,
      name,
      description,
    })

    setSpaces((current) => current.map((item) => (item.id === updated.id ? updated : item)))
  }

  const saveQuestionField = async (
    question: Question,
    field: 'question' | 'option_a' | 'option_b' | 'option_c' | 'option_d',
    nextValue: string,
  ) => {
    const updated = await invoke<Question>('modify_question', {
      id: question.id,
      questionInput: {
        question: field === 'question' ? nextValue : question.question,
        option_a: field === 'option_a' ? nextValue : question.option_a,
        option_b: field === 'option_b' ? nextValue : question.option_b,
        option_c: field === 'option_c' ? nextValue : question.option_c,
        option_d: field === 'option_d' ? nextValue : question.option_d,
        correct_answer: question.correct_answer,
        explanation: question.explanation,
        space_id: question.space_id,
      },
    })

    setQuestions((current) => current.map((item) => (item.id === updated.id ? updated : item)))
    setAllQuestions((current) => current.map((item) => (item.id === updated.id ? updated : item)))
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
                  <div className={`question-card-head${isManagingQuestions ? ' is-managing' : ''}`}>
                    {isManagingQuestions && (
                      <label className="question-select" aria-label={`Select question ${item.id}`}>
                        <input
                          type="checkbox"
                          checked={selectedQuestionIds.includes(item.id)}
                          onChange={() => toggleSelectedQuestion(item.id)}
                        />
                      </label>
                    )}

                    <div
                      className="question-toggle"
                      onClick={() => toggleExpanded(item.id)}
                      role="button"
                      tabIndex={0}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault()
                          toggleExpanded(item.id)
                        }
                      }}
                      aria-expanded={expandedIds.includes(item.id)}
                      aria-controls={`question-details-${item.id}`}
                    >
                      <div className="question-summary">
                        <span className="question-id">#{item.id}</span>
                        <div onClick={(event) => event.stopPropagation()}>
                          <EditableText
                            value={item.question}
                            onSave={(next) => saveQuestionField(item, 'question', next)}
                            disabled={isManagingQuestions || isDeletingQuestions}
                            className="question-inline-text question-inline-text-title"
                            inputClassName="question-inline-input question-inline-input-title"
                            as="h2"
                            aria-label={`Edit question ${item.id}`}
                          />
                        </div>
                      </div>
                      <ChevronDown
                        className={`question-chevron${expandedIds.includes(item.id) ? ' is-open' : ''}`}
                        aria-hidden="true"
                      />
                    </div>
                  </div>

                  {expandedIds.includes(item.id) && (
                    <div className="question-details" id={`question-details-${item.id}`}>
                      <ul className="question-options">
                        {([
                          { key: 'A' as const, value: item.option_a, field: 'option_a' as const },
                          { key: 'B' as const, value: item.option_b, field: 'option_b' as const },
                          { key: 'C' as const, value: item.option_c, field: 'option_c' as const },
                          { key: 'D' as const, value: item.option_d, field: 'option_d' as const },
                        ]).map((option) => (
                          <li
                            key={option.key}
                            className={`question-option${isCorrectOption(option.value, option.key, item) ? ' is-correct' : ''}`}
                          >
                            <span className="question-option-key">{option.key}</span>
                            <EditableText
                              value={option.value}
                              onSave={(next) => saveQuestionField(item, option.field, next)}
                              disabled={isManagingQuestions || isDeletingQuestions}
                              className="question-inline-text"
                              inputClassName="question-inline-input"
                              aria-label={`Edit option ${option.key} for question ${item.id}`}
                            />
                          </li>
                        ))}
                      </ul>

                      <p className="question-model">
                        Generated by: {item.model ?? 'Unknown model'}
                      </p>
                    </div>
                  )}
                </article>
              ))}
            </section>
          )}

          {isManagingQuestions && questions.length > 0 && (
            <section className="questions-manage-actions" aria-label="Question management actions">
              <button
                type="button"
                className="btn-primary btn-delete-questions"
                onClick={deleteSelectedQuestions}
                disabled={selectedQuestionIds.length === 0 || isDeletingQuestions}
              >
                {isDeletingQuestions
                  ? 'Deleting...'
                  : selectedQuestionIds.length <= 1
                    ? 'Delete selected question'
                    : `Delete ${selectedQuestionIds.length} selected questions`}
              </button>
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

                      <div className="recall-space-inline-editor" onClick={(event) => event.stopPropagation()}>
                        <EditableText
                          value={space.name}
                          onSave={(next) => saveSpaceField(space, 'name', next)}
                          disabled={deletingSpaceId !== null}
                          className="editable-space-title"
                          inputClassName="recall-space-inline-input recall-space-inline-title"
                          as="h2"
                          aria-label={`Edit title for ${space.name}`}
                        />
                      </div>

                      <div className="recall-space-inline-editor" onClick={(event) => event.stopPropagation()}>
                        <EditableText
                          value={space.description?.trim() ?? ''}
                          onSave={(next) => saveSpaceField(space, 'description', next)}
                          disabled={deletingSpaceId !== null}
                          placeholder="No description provided."
                          className="editable-space-description"
                          inputClassName="recall-space-inline-input"
                          as="p"
                          allowEmpty
                          aria-label={`Edit description for ${space.name}`}
                        />
                      </div>
                    </div>

                    <div className="recall-space-count" aria-label={`${questionCount} questions`}>
                      <strong>{questionCount}</strong>
                      <span>{questionCount === 1 ? 'Question' : 'Questions'}</span>
                    </div>
                  </div>

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
        </>
      )}
    </div>
  )
}
