# Default generation quantity and purpose

Default generation accepts `GenerationOptions` in `start_preview_generation`.
The maximum defaults to **20 learning items** and the purpose to **Balanced**.
Omitted fields receive these defaults. Counts must be positive integers within
the backend's `u32` range; purposes are a closed enum. Options are captured when
the job starts. One flashcard with an optional MCQ counts as one learning item.

Deep thinking keeps its existing command and generation behavior. There is no
Auto, unlimited, custom learning-focus prompt, or generated-rubric option.

## Whole-note flow

1. Finish Stage A extraction for every original chunk.
2. Retain verbatim knowledge points as candidates with backend identities and
   authoritative note, section, chunk, and original point-position mappings.
3. Assess one candidate per provider request, with a reused note outline,
   section heading, purpose description, anchored rubric, and other points from
   the same chunk. The scoring model receives no identities, maximum, weights,
   previous scores, or shortlist.
4. Reassess once with the original source chunk if source context is requested.
   Invalid, unresolved, and unsuitable assessments are excluded with diagnostics.
   Existing structured retries apply to each request; there are no fallback scores.
5. Compute fixed integer weighted scores and select one strongest representative
   per substantive section when the budget permits. When sections outnumber
   slots, choose the strongest representatives. Fill remaining slots by the
   same fixed ranking, respecting the global maximum and four points per chunk.
6. Send only selected points to Stage B, grouped by their original chunks. Local
   `kp_N` IDs map explicitly back to selected candidates and original source text.
   Existing content, ID, duplicate-output, and allocation validation apply.
7. Review and save through the existing learning-item workflow. Generation
   omissions or failures do not trigger automatic replenishment.

Scores tie by stable source order, independently of response completion order.
The selector never compares candidate text or changes scores as it fills slots.
Distinct contribution is a fixed **local** rubric score. Cross-chunk semantic
deduplication, overlap penalties, embeddings, and extra reranking are absent.

## Initial policy: `default-importance-v2`

| Purpose | Centrality | Explanatory | Foundational | Distinct | Application |
| --- | ---: | ---: | ---: | ---: | ---: |
| Balanced | 25 | 20 | 25 | 15 | 15 |
| Foundations | 20 | 10 | 45 | 15 | 10 |
| Explanations | 20 | 45 | 10 | 15 | 10 |
| Practical application | 20 | 15 | 10 | 15 | 40 |

Each rating is an integer from 1 through 5. Weights sum to 100 and scores range
from 100 to 500. These percentages are initial product defaults requiring
evaluation, not scientifically calibrated values. Foundations explicitly
includes essential definitions, prerequisites, and core principles.

Every assessment returns all five ratings in the same request. Application value
measures supported usefulness for procedures, decisions, and troubleshooting;
explanatory value measures mechanisms, causes, and rationale. Neither implies the
other, although a concept can score highly in both. The model must not invent
practical uses absent from the note evidence. Practical application gives the
application rating its strongest weight. This changes selection only: Stage B
still uses its existing question-generation prompt without a purpose parameter.

## Progress and diagnostics

Default jobs report extraction, assessment, selection, and generation as separate
phases. Progress is monotonic; phase percentages indicate completed work rather
than an estimate of elapsed time or cost. Original source counts remain separate
from assessment counts and selected/generation counts.

`default_selection` in progress and summary captures options, policy version,
assessment outcomes, section coverage, allocation, generated parent count, and
shortfall reasons. These data, candidates, and source chunks stay in job memory.
Saved learning-item schema, schedules, and review history are unchanged. Graph
payloads omit this Default-only field.

The UI explains the ceiling and distinguishes selected concepts from generated
items. Shortfalls distinguish extraction failures, unresolved assessments,
insufficient eligible concepts, per-chunk capacity, generation failures, omitted
items, and interrupted generation. A full selected allocation with omissions
does not imply insufficient source content.

## Verification and evaluation boundary

Local provider fixtures exercise the entire Default flow, structured retry,
source recovery, selected-only Stage B requests, out-of-order completion,
pause/cancel in each asynchronous phase, partial failures, and paired variants.
Pure selector tests cover purpose weights, section coverage, long notes, ties,
capacity, repeated headings/text, and shortfall attribution. Frontend tests cover
validation, immutable requests, graph arguments, and existing review/save inputs.

Assessing the whole note adds one request per candidate, plus bounded source
recovery and structured retries. A lower maximum reduces selected Stage B work,
but does not skip extraction or assessment. Do not infer reduced total cost.
Live purpose alignment, coverage, request count, and elapsed time still require
a reachable configured provider and representative notes.

## Backend organization

Default orchestration, candidate sources, and selection policy live together in
`src-tauri/src/services/generation/default/`. The shared `job.rs`, `progress.rs`,
and `save.rs` modules own job controls, preview contracts, and reviewed-draft
persistence respectively.

`services::generation` retains its public entry points for Tauri commands and
the standalone evaluation crate. Graph modules live under `generation/graph/`.
Desktop graph jobs run through `graph/desktop.rs`, using the shared job controls
and progress contracts. Evaluation tools call the stages in `graph/pipeline.rs`
directly. The generation facade only declares modules and exports entry points.
Shared provider requests and structured retries remain under `services/llm/`.
This organization does not change selection, source mapping, or saved data.

## Changed files

| Area | Files |
| --- | --- |
| Options and IPC | `src-tauri/src/models/generation_options.rs`, `src-tauri/src/models/mod.rs`, `src-tauri/src/lib.rs` |
| Candidate sources and fixed selection | `src-tauri/src/services/generation/default/candidates.rs`, `src-tauri/src/services/generation/default/selection.rs`, `src-tauri/src/services/mod.rs` |
| Default orchestration and provider integration | `src-tauri/src/services/generation/default/mod.rs`, `src-tauri/src/services/generation/default/pipeline.rs`, `src-tauri/src/services/llm/importance.rs`, `src-tauri/src/services/llm/default_generation.rs`, `src-tauri/src/services/llm.rs` |
| Generation facade and shared infrastructure | `src-tauri/src/services/generation/mod.rs`, `src-tauri/src/services/generation/job.rs`, `src-tauri/src/services/generation/progress.rs`, `src-tauri/src/services/generation/save.rs` |
| Graph organization and evaluation imports | `src-tauri/src/services/generation/graph/`, `eval/src/bin/graph_stage_a_eval.rs`, `eval/src/bin/graph_e2e_eval.rs`, `eval/src/bin/entity_resolution_eval.rs` |
| Study plan organization | `src-tauri/src/services/study_plan/` |
| Shared frontend contract and state | `src/generation/types.ts`, `src/generation/options.ts`, `src/generation/context.ts`, `src/generation/GenerationContext.tsx` |
| Setup and reporting UI | `src/pages/HomePage.tsx`, `src/generation/components/DefaultGenerationOptions.tsx`, `src/generation/components/GenerationProgress.tsx`, `src/components/DefaultGenerationMetrics.tsx`, `src/styles/generation.css` |
| Automated tests | `src-tauri/src/services/generation/tests.rs`, `src-tauri/src/services/generation/default/pipeline/tests.rs`, `src-tauri/src/services/generation/graph/desktop/tests.rs`, `scripts/test-generation-options.mjs`, `package.json` (test script), plus colocated Rust unit tests |
| Documentation | `docs/default-generation-selection.md` |

## Verification on 2026-10-03

- `npm run lint`: passed.
- `npm run build`: passed.
- `npm run test:recall`: 15 passed.
- `npm run test:generation-options`: 7 passed.
- `cargo test --manifest-path src-tauri/Cargo.toml`: 322 passed after adding
  application value; Rust test execution completed in 4.28 seconds, excluding compilation.
- `git diff --check`: passed.
- Browser verification used the running frontend with isolated IPC fixtures,
  at the configured 800 × 600 window size and a narrower viewport. Checked
  inline numeric validation, purpose descriptions, keyboard focus, mode
  isolation, captured options, all four phases, pause/resume, completion,
  generation omissions, review, and the save UI. No real database writes were
  made by that fixture. Native desktop interaction was not tested.
- The bounded two-chunk provider fixtures made 9 requests for a maximum of one
  and 10 for a maximum of two: two extraction requests, six assessment requests,
  and one or two Stage B requests. Coverage and correct original point mapping
  were asserted; these are fixture results, not live model quality or cost data.
- Live quality evaluation was blocked: the saved provider's configured local
  endpoint refused the connection. Live purpose alignment and coverage remain
  unmeasured; weights are not claimed to be calibrated.

The ignored `eval/output/generation-ui-check.html` fixture and
`eval/output/generation-quantity-shortfall.jpg` screenshot are local verification
artifacts, excluded from the source change manifest.
