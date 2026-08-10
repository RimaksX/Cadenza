# Database schema

Normative table and column definitions: `PROJECT_MASTER.json`, section
`7_Модель_данных`. Do not restate them here — this file records how the schema is
applied and evolved.

Status: created in M2 as twelve append-only migrations,
`crates/infra/src/db/migrations/m0001_initial.rs` .. `m0012_review.rs`, with a
thirteenth added in M5.

Three columns from section 7 are deliberately not created, and every instant is
stored as an integer rather than text. The reasoning is in
[MASTER_ISSUES.md](MASTER_ISSUES.md) findings 4, 5, 15 and 16, and in the module
documentation of `crates/infra/src/db/mod.rs`.

One table is not in section 7 at all. `profile_track_genres`, with
`profile_tracks.genres_overridden` beside it, is what lets one listener correct a
genre without correcting it for everyone who shares the file — section 2.1
against 12.1, finding 10. Section 7.2 should gain both.

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
