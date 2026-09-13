# Git workflow

* Use lowercase kebab-case branch names following `<type>/<name>`.
* Use these branch types:

  * `feat/` — new features
  * `fix/` — bug fixes
  * `refactor/` — code restructuring
  * `ci/` — CI/CD changes
  * `docs/` — documentation
  * `test/` — test changes
  * `chore/` — maintenance
* Examples: `feat/entity-resolution`, `fix/updater-error`, `refactor/generation-service`.
* Open pull requests into `main`.
* Use Conventional Commits, e.g. `feat: add session results dashboard`.
* Never commit secrets, signing keys, or `.env` files.
* Before committing frontend work, run `npm run lint` and `npm run build`.
* Before committing Rust work, run `cargo test --manifest-path src-tauri/Cargo.toml`.
* Use `npm run version:set -- X.Y.Z` for release version bumps.

# Database migrations

* Never modify a released migration; add a new timestamped migration.
* Test fresh installation and upgrades from every supported schema anchor.
* Add populated fixtures only for major schema eras or risky data transformations.
* Preserve user content, schedules, provenance, and historical reviews.
* Follow `src-tauri/migrations/README.md` when changing the database schema.
