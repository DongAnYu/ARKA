# A.R.K.A

**Active Recall Knowledge Assistant**

[![Version](https://img.shields.io/badge/version-0.1.0-2563eb)](https://github.com/DongAnYu/A.R.K.A./releases)
[![License: PUL 1.0](https://img.shields.io/badge/license-PUL%201.0-7c3aed.svg)](LICENSE)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri-24c8db.svg)](https://tauri.app/)

A.R.K.A is a local-first desktop study application that turns Markdown notes into multiple-choice questions and schedules them for active-recall review. It supports both a conventional chunk-based generation pipeline and a knowledge-graph pipeline designed to produce questions that connect related concepts.

> [!NOTE]
> A.R.K.A is currently an MVP. Data models, generation behavior, and user-facing workflows may change before version 1.0.

A.R.K.A is source-available under the [A.R.K.A Personal Use Licence 1.0](LICENSE). It is not open-source software. Personal and other non-commercial study, teaching, research, charitable, volunteer, and community uses are permitted. Commercial use and distribution of independent modified versions require separate written permission.

## Features

- Load Markdown notes directly from a local vault or folder.
- Generate MCQs with either chunk-based or knowledge-graph generation.
- Preview generation progress and pause or cancel long-running jobs.
- Review generated questions before saving them.
- Organize questions into recall spaces.
- Run focused review sessions with SM-2 spaced-repetition scheduling.
- Use local Ollama models or OpenRouter-compatible hosted models.
- Keep notes, questions, settings, and scheduling data in a local SQLite database.
- Evaluate generation pipelines with JSON and XLSX reports.

## How It Works

1. Select a folder containing Markdown notes.
2. Choose a note and a generation strategy.
3. A.R.K.A extracts knowledge and generates candidate MCQs.
4. Inspect the preview and save useful questions to a recall space.
5. Complete review sessions and rate each answer to update its SM-2 schedule.

## Installation

### Download a release

Packaged installers are published on the [GitHub Releases](https://github.com/DongAnYu/A.R.K.A./releases) page. Windows is the primary supported platform for the MVP.

### Build from source

Prerequisites:

- Node.js 20 or later
- Rust with Cargo
- Platform prerequisites for [Tauri 2](https://v2.tauri.app/start/prerequisites/)
- Windows Build Tools with the MSVC toolchain when building on Windows

Clone and run the application:

```bash
git clone https://github.com/DongAnYu/A.R.K.A..git arka
cd arka
npm install
npm run tauri:dev
```

## Model Configuration

Open the **Models** page in A.R.K.A and choose one of the supported providers:

### Ollama

Use an Ollama server for local model inference. Configure its base URL, fetch the available models, and select the model used for generation.

### OpenRouter

Enter your own OpenRouter API key and model ID. The key is saved in the application's local database and must never be committed to the repository.

When using a hosted provider, relevant note content is sent to that provider to generate questions. Ollama keeps inference local when it is running on the same device.

## Development

| Command | Purpose |
| --- | --- |
| `npm run tauri:dev` | Run the complete desktop application in development mode |
| `npm run dev` | Run only the Vite frontend |
| `npm run build` | Type-check and build the frontend |
| `npm run lint` | Run ESLint |
| `npm run tauri:build` | Build release installers and application bundles |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Run the Rust test suite |

## Packaging

Create a production desktop bundle with:

```bash
npm run tauri:build
```

On Windows, Tauri writes the installer artifacts beneath `src-tauri/target/release/bundle/`, including MSI and NSIS packages when the required tooling is available.

## Project Structure

```text
arka/
|-- src/                         React and TypeScript frontend
|-- src-tauri/
|   |-- src/                     Rust application and domain services
|   |-- migrations/              SQLite migrations
|   `-- tauri.conf.json          Desktop application and bundle settings
|-- docs/                        Project and evaluation notes
|-- eval/                        Evaluation documentation and reports
`-- package.json                 Frontend, evaluation, and Tauri scripts
```

## Privacy and Security

- Local notes and generated study data are not uploaded by A.R.K.A unless a remote model provider is selected.
- Remote model providers receive the note excerpts required for generation and apply their own privacy policies.
- Never commit `.env` files, API keys, vault contents, or generated evaluation artifacts containing private notes.
- Review generated questions before relying on them for study; model output can be incomplete or incorrect.

## Contributing

Issues and pull requests are welcome.

1. Fork the repository and create a focused feature branch.
2. Keep changes consistent with the existing React, Tauri, and Rust patterns.
3. Run `npm run build`, `npm run lint`, and the Rust test suite.
4. Describe behavioral changes and verification steps in the pull request.

Please do not include private notes, API keys, or generated artifacts containing sensitive material in bug reports.

By submitting a contribution, you agree to the contributor licence grant in Section 3 of the Personal Use Licence.

## License

Copyright (c) 2026 Dongan Yu. All rights reserved.

A.R.K.A is distributed under the [A.R.K.A Personal Use Licence 1.0](LICENSE), a source-available licence that permits personal use, non-commercial educational and community use, private modifications, sharing unmodified copies, and contribution forks. Commercial use and independent modified distributions require a separate written licence from the Copyright Holder.
