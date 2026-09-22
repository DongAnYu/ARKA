# A.R.K.A

**Active Recall Knowledge Assistant**

[![GitHub Release](https://img.shields.io/github/v/release/DongAnYu/ARKA?include_prereleases)](https://github.com/DongAnYu/ARKA/releases)
[![CI](https://github.com/DongAnYu/ARKA/actions/workflows/ci.yml/badge.svg)](https://github.com/DongAnYu/ARKA/actions/workflows/ci.yml)
[![License: PUL 1.0](https://img.shields.io/badge/license-PUL%201.0-7c3aed.svg)](LICENSE)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri-24c8db.svg)](https://tauri.app/)

A.R.K.A is a local-first Windows desktop study application that turns Markdown notes into editable flashcards and multiple-choice questions, then schedules them for active-recall review. Its daily study plan prioritizes due reviews before introducing new material, so learners can focus on studying instead of managing a queue.

Choose fast, focused generation for an individual note or use **Deep thinking** to build a knowledge graph and generate questions that connect related concepts. Every generated item can be reviewed, edited, kept, or discarded before it enters the study library.

> [!NOTE]
> A.R.K.A is an evolving MVP. Data models, generation behavior, and user-facing workflows may change before version 1.0.

A.R.K.A is source-available, not open-source software. The [A.R.K.A Personal Use Licence 1.0](LICENSE) permits personal and other non-commercial study, teaching, research, charitable, volunteer, and community uses. Commercial use and distribution of independent modified versions require separate written permission.

## Highlights

- Load Markdown notes directly from a local vault or folder.
- Generate editable flashcards and multiple-choice questions from note content.
- Choose **Default generation** for speed or **Deep thinking** for knowledge-graph-assisted questions.
- Follow generation from the app title bar and pause or cancel long-running jobs.
- Review, edit, keep, or discard generated Learning Items before saving them.
- Organize Learning Items into Recall Spaces.
- Search and filter the Library by review status or item format, and manage items in bulk.
- Work through an adjustable daily plan that schedules due reviews before New items.
- Review with SM-2 spaced-repetition scheduling using mouse or keyboard controls.
- Use Ollama locally or connect OpenAI and OpenRouter-compatible hosted models.
- Keep notes, Learning Items, settings, study plans, and review history in local SQLite storage.

## How It Works

1. Select a folder containing Markdown notes and choose a note to study.
2. Choose Default generation or Deep thinking and confirm the required models are ready.
3. Review the generated Learning Items, edit them if needed, and select what to keep.
4. Save the selected items to a Recall Space.
5. Open Recall and follow today's plan. Due reviews are presented before New material.
6. Answer multiple-choice questions or reveal and rate flashcards to update their SM-2 schedules.

## Daily Study Plan

Recall builds one manageable plan for the day across all Recall Spaces:

- **Daily study target** controls the total number of planned items.
- **Maximum New items per day** limits how much unseen material can enter the plan and may be set to zero.
- Scheduled reviews take priority over New items.
- Filtering by Recall Space focuses the session without creating a separate quota.
- Progress and the remaining review/New workload are visible from Recall and the Library.

Study preferences are stored locally and can optionally be applied to the remainder of today's plan.

## Installation

### Download a release

Packaged installers are published on the [GitHub Releases](https://github.com/DongAnYu/ARKA/releases) page. Windows is the primary supported platform for the MVP.

### Build from source

Prerequisites:

- Node.js 22, matching [`.nvmrc`](.nvmrc)
- Rust with Cargo
- Platform prerequisites for [Tauri 2](https://v2.tauri.app/start/prerequisites/)
- Windows Build Tools with the MSVC toolchain when building on Windows

Clone and run the application:

```bash
git clone https://github.com/DongAnYu/ARKA.git arka
cd arka
npm ci
npm run tauri:dev
```

## Model Configuration

Open **Models** in A.R.K.A to configure the two model roles independently:

| Model role | Purpose | Required for |
| --- | --- | --- |
| Question generation | Creates flashcards and multiple-choice questions | Default generation and Deep thinking |
| Entity embeddings | Finds related concepts while building the knowledge graph | Deep thinking |

Supported providers:

| Provider | Processing | Configuration |
| --- | --- | --- |
| Ollama | Local when Ollama runs on the same device | Base URL and an installed model |
| OpenAI | Hosted | Base URL, model ID, and API key |
| OpenRouter | Hosted | Base URL, model ID, and API key |

A.R.K.A can fetch installed models from an Ollama connection and test the embedding configuration before generation. Generation and embedding settings are saved together in the application's local database.

When a hosted provider is selected, the note content required for generation is sent to that provider and is subject to its privacy policy. Never commit API keys to the repository.

## Privacy and Security

- Notes, Learning Items, settings, schedules, and review history are stored locally in SQLite.
- Local notes are not uploaded by A.R.K.A unless a hosted model provider is selected.
- Hosted providers receive the note excerpts required for the requested generation operation.
- API keys are stored in the application's local database and must not be committed or shared.
- Never include private notes, vault contents, or generated artifacts containing sensitive material in issues or evaluation reports.
- Review generated learning material before relying on it; model output can be incomplete or incorrect.

## Development

| Command | Purpose |
| --- | --- |
| `npm run tauri:dev` | Run the complete desktop application in development mode |
| `npm run dev` | Run only the Vite frontend |
| `npm run lint` | Run ESLint |
| `npm run build` | Type-check and build the frontend |
| `npm run test:recall` | Run deterministic Recall behavior tests |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Run the Rust test suite |
| `npm run tauri:build` | Build release installers and application bundles |

### Generation evaluation

The repository includes repeatable evaluation commands that write JSON and XLSX reports beneath `eval/output/`:

| Command | Evaluation |
| --- | --- |
| `npm run eval:default` | Default generation pipeline |
| `npm run eval:graph-stage-a` | Knowledge-graph Stage A extraction |
| `npm run eval:graph-e2e` | End-to-end graph generation |
| `npm run eval:entity-resolution` | Entity-resolution fixture |
| `npm run eval:entity-resolution:hard` | Hard entity-resolution fixture |

Generated evaluation output is ignored by Git and may contain material derived from the source note. Inspect it before sharing.

## Packaging

Create a production desktop bundle with:

```bash
npm run tauri:build
```

On Windows, Tauri writes installer artifacts beneath `src-tauri/target/release/bundle/`, including MSI and NSIS packages when the required tooling is available.

## Project Structure

```text
arka/
|-- src/                         React and TypeScript frontend
|-- src-tauri/
|   |-- src/                     Rust application and domain services
|   |-- migrations/              SQLite migrations
|   `-- tauri.conf.json          Desktop application and bundle settings
|-- docs/                        Project and evaluation notes
|-- eval/                        Evaluation tooling, fixtures, and reports
|-- scripts/                     Repository test and release utilities
`-- package.json                 Frontend, evaluation, and Tauri scripts
```

## Contributing

Issues and pull requests are welcome.

1. Fork the repository and create a focused feature branch.
2. Keep changes consistent with the existing React, Tauri, Rust, and migration patterns.
3. Run `npm run lint`, `npm run build`, and the relevant frontend and Rust tests.
4. Describe user-visible behavior, data impact, and verification steps in the pull request.

Do not include private notes, API keys, vault contents, or generated artifacts containing sensitive material in bug reports or pull requests.

By submitting a contribution, you agree to the contributor licence grant in Section 3 of the Personal Use Licence.

## License

Copyright (c) 2026 Dongan Yu. All rights reserved.

A.R.K.A is distributed under the [A.R.K.A Personal Use Licence 1.0](LICENSE), a source-available licence that permits personal use, non-commercial educational and community use, private modifications, sharing unmodified copies, and contribution forks. Commercial use and independent modified distributions require a separate written licence from the Copyright Holder.
