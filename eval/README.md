# Evaluation

This folder contains evaluation tooling for the active-recall generation pipelines.

## Default target

- `docs/evaluation_notes/Photosynthesis.md`

Artifacts are written to `eval/output/`.

## Commands

From the repository root:

```powershell
npm run eval:default
npm run eval:graph-stage-a
npm run eval:graph-e2e
npm run eval:entity-resolution
npm run eval:entity-resolution:hard
```

## Required environment for evaluation providers

- `LLM_MODEL`
- `OPENAI_API_KEY` or `OPENROUTER_API_KEY` for remote LLM providers
- `EMBEDDING_MODEL` for entity-resolution and graph E2E evaluation
- Optional: `LLM_PROVIDER` (`ollama`, `openai`, or `openrouter`; default `openrouter`)
- Optional: `EMBEDDING_PROVIDER` (`ollama`, `openai`, or `openrouter`; default `ollama`)
- Optional: `LLM_BASE_URL`, `OPENAI_BASE_URL`, or `OPENROUTER_BASE_URL`
- Optional: `EMBEDDING_BASE_URL`
- Optional: `LLM_TIMEOUT_SECS`
- Optional: `EMBEDDING_TIMEOUT_SECS`

The entity-resolution and graph E2E evaluators use `LLM_PROVIDER` (default:
`openrouter`) and
`EMBEDDING_PROVIDER` (default: `ollama`). Remote providers use
`OPENAI_API_KEY` or `OPENROUTER_API_KEY`; `LLM_API_KEY` and
`EMBEDDING_API_KEY` can override the provider-specific key for their respective
service. Optional settings include `LLM_BASE_URL`, `EMBEDDING_BASE_URL`,
`LLM_TIMEOUT_SECS`, and `EMBEDDING_TIMEOUT_SECS`.

## Evaluators

`eval:default` runs the legacy AQG pipeline and writes:

- `photosynthesis-*.json`
- `photosynthesis-*.xlsx`

`eval:graph-stage-a` runs graph Stage A only: chunking, entity/knowledge-point extraction, graph consolidation, and graph validation. It writes:

- `stage-a-photosynthesis-*.json`
- `stage-a-photosynthesis-*.xlsx`

`eval:graph-e2e` runs the same graph order used by the application: Stage A,
semantic entity resolution, index rebuilding, and Stage B from the rewritten
graph. Stage A prints per-chunk request/completion timing and provider retries.
During resolution it prints every embedding candidate and score, followed by
completion-ordered LLM verifier decisions and active-request counts. It writes:

- `graph-e2e-photosynthesis-*.json`
- `graph-e2e-photosynthesis-*.xlsx`

The E2E JSON includes the full Stage A result, resolved graph, candidate scores,
verifier reasons, verification progress, merge groups, stage timings, full Stage
B result, and compact summaries. The spreadsheet includes Summary, Stage A
Chunks, Entities Before, Entity Resolution, Verification Progress, Entities
After, and Stage B MCQs sheets. Candidate settings can be overridden with
`--minimum-similarity`, `--max-candidates`, `--max-context-points`, and
`--max-concurrency`. Stage A uses bounded concurrency 10 by default and can be
overridden with `--stage-a-concurrency`.

`eval:entity-resolution` runs the fixed entity-resolution fixture through the
production context, embedding, candidate, verifier, merge, rewrite, and index
pipeline. It writes a timestamped JSON report containing candidate recall,
expected merges, false-positive rate, unresolved positive pairs, entity counts,
merge groups, candidate similarities, and verifier reasons. Final graph
resolution and strict direct-pair retrieval are reported separately because a
multi-member cluster can merge correctly through transitive verified edges.

`eval:entity-resolution:hard` runs a larger adversarial fixture. It adds
three-member synonym clusters, ambiguous abbreviations, Unicode and punctuation
variants, and related-but-distinct processes, properties, compounds, and
locations. Because each entity can propose up to three candidates, this run can
make substantially more verifier requests than the smoke fixture.
