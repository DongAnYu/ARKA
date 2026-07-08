import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { ArrowLeft, ChevronDown } from 'lucide-react'

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
    setError('')
  }

  const getSpaceQuestionCount = (spaceId: number) => {
    return allQuestions.filter((item) => item.space_id === spaceId).length
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
                  <button
                    type="button"
                    className="question-toggle"
                    onClick={() => toggleExpanded(item.id)}
                    aria-expanded={expandedIds.includes(item.id)}
                    aria-controls={`question-details-${item.id}`}
                  >
                    <div className="question-summary">
                      <span className="question-id">#{item.id}</span>
                      <h2>{item.question}</h2>
                    </div>
                    <ChevronDown
                      className={`question-chevron${expandedIds.includes(item.id) ? ' is-open' : ''}`}
                      aria-hidden="true"
                    />
                  </button>

                  {expandedIds.includes(item.id) && (
                    <div className="question-details" id={`question-details-${item.id}`}>
                      <ul className="question-options">
                        {[
                          { key: 'A' as const, value: item.option_a },
                          { key: 'B' as const, value: item.option_b },
                          { key: 'C' as const, value: item.option_c },
                          { key: 'D' as const, value: item.option_d },
                        ].map((option) => (
                          <li
                            key={option.key}
                            className={`question-option${isCorrectOption(option.value, option.key, item) ? ' is-correct' : ''}`}
                          >
                            <span className="question-option-key">{option.key}</span>
                            <span>{option.value}</span>
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
        <section className="recall-spaces-grid" aria-label="Recall spaces list">
          {spaces.map((space) => {
            const questionCount = getSpaceQuestionCount(space.id)

            return (
              <article className="recall-space-card" key={space.id}>
                <button
                  type="button"
                  className="recall-space-button"
                  onClick={() => openSpace(space)}
                  aria-label={`Open ${space.name}`}
                >
                  <div className="recall-space-meta">
                    <span className="question-id">Space #{space.id}</span>
                    <h2>{space.name}</h2>
                    <p>{space.description?.trim() || 'No description provided.'}</p>
                  </div>

                  <div className="recall-space-count" aria-label={`${questionCount} questions`}>
                    <strong>{questionCount}</strong>
                    <span>{questionCount === 1 ? 'Question' : 'Questions'}</span>
                  </div>
                </button>
              </article>
            )
          })}
        </section>
      )}
    </div>
  )
}