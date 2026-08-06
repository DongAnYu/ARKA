# Project Structure (MVP)

## Current scaffold

- React + TypeScript frontend at `src/`
- Tauri Rust backend at `src-tauri/`
- SQLite migrations at `src-tauri/migrations/`

## Recommended module layout (next step)

```text
arka/
  src/
    app/
    features/
      quiz/
      settings/
    components/
    lib/
  src-tauri/
    src/
      app_state.rs
      commands/
        mod.rs
        settings.rs
        quiz.rs
        vault.rs
      services/
        db.rs
        markdown.rs
        obsidian_scan.rs
        llm.rs
        scheduler.rs
      models/
        note.rs
        question.rs
        settings.rs
      lib.rs
      main.rs
    migrations/
      001_init.sql
    tauri.conf.json
  docs/
    PROJECT_STRUCTURE.md
```

## Why this structure

- `commands/`: Tauri `#[tauri::command]` functions exposed to React
- `services/`: business logic and integrations (SQLite, Markdown, LLM, scanning)
- `models/`: serializable types shared across services and commands
- `features/`: frontend slices aligned with user flows (quiz/settings)
