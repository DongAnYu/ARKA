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
```

## Required environment

- `LLM_MODEL`
- `OPENROUTER_API_KEY`
- Optional: `OPENROUTER_BASE_URL`
- Optional: `LLM_TIMEOUT_SECS`

The evaluators force the runtime LLM provider to OpenRouter.

## Evaluators

`eval:default` runs the legacy AQG pipeline and writes:

- `photosynthesis-*.json`
- `photosynthesis-*.xlsx`

`eval:graph-stage-a` runs graph Stage A only: chunking, entity/knowledge-point extraction, graph consolidation, and graph validation. It writes:

- `stage-a-photosynthesis-*.json`
- `stage-a-photosynthesis-*.xlsx`

`eval:graph-e2e` runs graph Stage A and graph Stage B together. It prints both Stage A and Stage B summaries, then writes:

- `graph-e2e-photosynthesis-*.json`
- `graph-e2e-photosynthesis-*.xlsx`

The E2E JSON includes the full Stage A result, full Stage B result, and compact summaries. The E2E spreadsheet includes Summary, Stage A Chunks, Entities, and Stage B MCQs sheets.
