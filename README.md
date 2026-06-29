# Obsidian Active Recall (MVP)

Windows desktop app using Tauri + Rust + React + TypeScript.

## Stack

- Rust backend (`src-tauri/`)
- Tauri desktop shell
- React + TypeScript frontend (`src/`)
- SQLite (local DB)
- OpenAI-compatible API for MCQ generation

## Quick Start

Prerequisites:

- Rust toolchain (`rustup`, `cargo`)
- Node.js 20+
- Windows Build Tools for Rust MSVC target

Run locally:

```bash
npm install
npm run tauri:dev
```

Build Windows app:

```bash
npm run tauri:build
```

## Useful Scripts

- `npm run dev` - Frontend only (Vite on port 1420)
- `npm run tauri:dev` - Full desktop app in development
- `npm run build` - Frontend production build
- `npm run tauri:build` - Desktop bundle build

## Current Project Layout

```text
obsidian-active-recall/
  src/
  src-tauri/
    src/
    migrations/
      001_init.sql
  docs/
    PROJECT_STRUCTURE.md
```

See `docs/PROJECT_STRUCTURE.md` for the recommended modular layout for the MVP features.
