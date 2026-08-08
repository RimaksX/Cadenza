# Database schema

Normative table and column definitions: `PROJECT_MASTER.json`, section
`7_Модель_данных`. Do not restate them here — this file records how the schema is
applied and evolved.

Status: no tables exist yet. The schema lands in M2 as numbered migrations
(`crates/infra/src/db/migrations/m0001_initial.rs` .. `m0012_review.rs`).

## Rules

- SQLite with WAL mode, foreign keys enabled, prepared statements.
- Every schema change goes through a new numbered migration. Migrations are append-only;
  never edit one that has shipped.
- Entity identifiers are UUIDs stored as `TEXT`.
- Global tables (`media_files`, `track_features`, `artists`, `albums`, `genres`,
  `analysis_jobs`, `app_settings`) hold physical and technical facts about files.
- Every table containing user data carries `profile_id`. Profile data is never shared
  across profiles.
- `play_events` and the `daily_*` aggregates are retained for 30 days when history is
  enabled, and are not written at all when it is disabled.

## Storage locations

`%APPDATA%/Cadenza/app.db`; artwork and analysis scratch live under
`%LOCALAPPDATA%/Cadenza/cache/` (section `6_Пути_данных`).
