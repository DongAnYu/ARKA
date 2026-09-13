# Database migration policy

ARKA stores user-authored learning content and review schedules in SQLite. Treat every migration as a data-preservation operation, not only a schema update.

## Migration rules

1. Keep migration history append-only. Never edit, rename, reorder, or delete a migration included in a release.
2. Add migrations as `YYYYMMDDHHMMSS_descriptive_name.sql`.
3. Let SQLx own migration ordering, checksums, and transactions.
4. Make destructive or transformational migrations fail closed when legacy data cannot be converted safely.
5. Preserve persistent IDs, recall-space membership, learning content, generation provenance, schedules, and historical reviews unless a documented product decision says otherwise.
6. Check foreign-key integrity and explicitly test intended cascade or restrict behavior.
7. Keep backups recoverable; never overwrite the original database automatically during recovery.

## Test strategy

Migration tests live in `src/services/database_migration_tests.rs`. Populated historical data lives in `tests/fixtures/migrations/`.

Fixtures contain data only. Their schema must come from the real migrations that shipped with the represented release. This prevents a handwritten historical schema from drifting away from the actual SQLx migration history.

Keep one populated fixture per important schema era, not one per application version. The current `v016_populated.sql` fixture anchors the MCQ-only schema that existed before LearningItems.

Every schema change must continue to pass:

- fresh database to current schema;
- repeated startup with no pending migrations;
- every supported historical anchor to current schema;
- SQLx checksum validation;
- current foreign-key and schema invariants.

Add a focused test when a migration transforms data, rebuilds a table, changes constraints or foreign keys, changes scheduling, or can fail after partially executing. Simple additive columns and indexes normally need a focused schema assertion, not another release fixture.

## Adding a future migration

1. Add a new timestamped SQL file.
2. Decide whether it is additive, transformational, or a new schema era.
3. For an additive change, extend the current-schema assertions where useful.
4. For a transformation, add representative data to the relevant historical fixture and assert the preserved result.
5. For a major schema era, add a new named fixture and a migration-boundary constant while retaining older supported anchors.
6. Derive current migration counts from `MIGRATOR`; do not hardcode totals that change whenever a migration is added.
7. Run `cargo test --manifest-path src-tauri/Cargo.toml`.

An anchor may be retired only after ARKA explicitly stops supporting direct upgrades from that schema era or provides a separate supported import path.
