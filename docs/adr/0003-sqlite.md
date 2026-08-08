# 3. SQLite for local storage

Status: accepted (M0)

## Context

A single-user local player storing 1000–5000 tracks plus playlists, queue, 30 days of
play history and daily aggregates. No server, no network, no sync (today).

## Decision

Embedded SQLite at `%APPDATA%/Cadenza/app.db`, in WAL mode, with foreign keys on,
prepared statements, and append-only numbered migrations under
`crates/infra/src/db/migrations/`.

## Consequences

- WAL lets background scanning and analysis write while the UI reads, which matters
  because the analysis worker runs continuously at low priority.
- Foreign keys make profile isolation enforceable by the database rather than by
  convention.
- Aggregates (`daily_track_stats` and friends) are maintained incrementally instead of
  recomputed, so the analytics dashboard stays cheap.
- Migrations are the only way the schema changes; a shipped migration is never edited.
- UUID `TEXT` keys cost a little space but keep the door open for future device sync.
