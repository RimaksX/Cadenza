# Database schema

Normative table and column definitions: `PROJECT_MASTER.json`, section
`7_Модель_данных`. Do not restate them here — this file records how the schema is
applied and evolved.

Status: eighteen append-only migrations,
`crates/infra/src/db/migrations/m0001_initial.rs` .. `m0018_radio_feedback.rs`.
Twelve were created in M2; the rest arrived with the milestone that needed them
— per-profile genres in M5, `queue_state` and `queue_entries` in M7, the
built-in equaliser presets and their tone controls in M9, the eight built-in
moods in M13, and the verdict column on `radio_session_items` in M13.

Three columns from section 7 are deliberately not created, and every instant is
stored as an integer rather than text. The reasoning is in
[MASTER_ISSUES.md](MASTER_ISSUES.md) findings 4, 5, 15 and 16, and in the module
documentation of `crates/infra/src/db/mod.rs`.

Three tables are not in section 7 at all. `queue_state` and `queue_entries`
(migration 14) are what "восстановление последней очереди" of section 2.3 and
"собственную очередь" of 2.5 require and section 7 never defines — finding 31.
Section 7 should gain both.

`profile_track_genres`, with
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
- `play_events` is retained for 30 days when history is enabled, and is not
  written at all when it is disabled. Retention is enforced by deleting at
  startup, for every profile, rather than by filtering queries.

## Two things the schema is ready for and nobody writes

`daily_track_stats`, `daily_artist_stats`, `daily_genre_stats` and
`daily_radio_stats` exist since migration 7 and are never written: the dashboard
counts straight out of `play_events`, which over thirty days of one listener is
cheaper than keeping a second copy of the same facts correct (finding 54).

`play_events.radio_session_id` is always `NULL`, so "how much of this station did
I listen to" cannot be answered yet. The column and its `CHECK` are right; what
is missing is that `QueueService` knows which station is playing and
`PlaybackService`, which writes the event, does not.

## Storage locations

`%APPDATA%/Cadenza/app.db`; artwork and analysis scratch live under
`%LOCALAPPDATA%/Cadenza/cache/` (section `6_Пути_данных`).
