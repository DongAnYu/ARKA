# AQG Evaluation

This folder contains end-to-end evaluation tooling for the question generation pipeline.

## Default target

- `docs/evaluation_notes/Photosynthesis.md`

## Run

From the repository root:

```powershell
npm run eval:aqg
```

Or run the script directly:

```powershell
powershell -ExecutionPolicy Bypass -File .\eval\run-aqg-eval.ps1
```

## Required environment

- `LLM_MODEL`
- `OPENROUTER_API_KEY`
- Optional: `OPENROUTER_BASE_URL`
- Optional: `LLM_TIMEOUT_SECS`

The evaluator forces the AQG pipeline to use the OpenRouter provider, then writes:

- A JSON artifact with chunk previews, key points, and flattened question rows
- An XLSX spreadsheet for manual review

Artifacts are written to `eval/output/`.