# Project plan

Milestones and their definition of done are specified in `PROJECT_MASTER.json`
section `11_План_реализации`. This file tracks progress only.

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold проекта | done |
| M1 | Core skeleton | done |
| M2 | SQLite infrastructure | done |
| M3 | Профили и настройки | done |
| M4 | Библиотека и сканирование | done |
| M5 | Базовый audio engine | done |
| M6 | UI shell | not started |
| M7 | Плейлисты, очередь, repeat/shuffle | not started |
| M8 | Crossfade и gapless | not started |
| M9 | Эквалайзер | not started |
| M10 | Визуализация | not started |
| M11 | DSP-анализ | not started |
| M12 | Smart shuffle | not started |
| M13 | Smart Radio | not started |
| M14 | Аналитика и retention | not started |
| M15 | Финальный дизайн | not started |
| M16 | Packaging и стабилизация | not started |

Milestones are implemented in order.

## M0 — what was actually built

Cargo workspace with five crates, pinned stable toolchain, `.gitignore`, CI running
fmt/clippy/build/test on `windows-latest`, documentation skeleton, and a runnable
`cadenza` binary.

Deliberately deferred (each arrives in the milestone that first needs it, rather than
as an empty stub now):

- the ~180 remaining source files listed in `5_Полная_структура_проекта`
- every third-party dependency: `rusqlite`, `slint`, `cpal`, `symphonia`, `rubato`,
  `rustfft`, `lofty`, `notify`, `directories`
- `deny.toml` — nothing to audit until there are dependencies
- `packaging/`, `resources/` — M15/M16

## M1 — what was actually built

The whole domain layer of `crates/core`, plus the application context: typed
identifiers, the error type, eight value objects, sixteen entities, eight
policies and twenty ports, with 105 unit tests.

Two additions to the file list in section 5, both because section 7 or 4.4 refers
to something section 5 gives no home: `domain/analysis.rs` for `analysis_jobs`,
and `domain/value_objects/timestamp.rs` for `ClockPort`'s return type.

`core` depends on `thiserror` and `uuid` and nothing else. No calendar crate: a
timestamp is unix milliseconds, and the local-date handling the daily aggregates
need arrives with them in M14.

Policies whose formulas the master file fixes are implemented and tested now —
history classification, retention, the previous-track threshold, the
source-driven transition rule, the transition score of 9.3, the similarity score
of 10.3, and the equaliser band layout. The two it leaves open, smart shuffle
selection and radio ranking, expose only their hard constraints and gain their
scoring in M12 and M13.

Deferred: `commands.rs`, `events.rs`, `dto.rs`, `view_state.rs`, `services/` and
`state/` — none are in M1's task list, and each belongs to the milestone that
first has something to put in it.

Contradictions found in the master file while doing this, and what was chosen for
each, are in [MASTER_ISSUES.md](MASTER_ISSUES.md). Two remain open: per-profile
genre overrides, and refreshing section 16's status.

## M2 — what was actually built

The whole schema of PROJECT_MASTER section 7, as twelve append-only migrations,
behind a connection pool. 17 integration tests against a real file.

- `db/sqlite.rs` — one place that opens a connection and applies the pragmas.
  WAL, foreign keys, `synchronous = NORMAL`, a 5 s busy timeout. Opening refuses
  to continue if WAL did not take effect.
- `db/pool.rs` — four connections, checkout blocks with a timeout rather than
  forever, guards return their connection on drop including during a panic.
- `db/migrations/` — the runner plus `m0001`..`m0012`. Each migration runs in a
  transaction with the row that records it, so a failure leaves nothing behind.
  Version numbering is checked at compile time; a database from a newer build is
  refused rather than half-read.
- `testkit/temp_db.rs` — a migrated database in a temp directory, deleted on
  drop. On disk rather than in memory, because an in-memory database cannot use
  WAL and would not exercise what production runs.

The schema does more than store rows: `CHECK` constraints reject unknown enum
values, gains outside ±12 dB, retention windows over 30 days, a listen that is
both completed and skipped, a built-in preset owned by a profile, and a duplicate
review that cannot say what it duplicates. Deleting a profile cascades to
everything scoped to it and leaves the shared catalogue alone.

Three corrections to section 7, all in [MASTER_ISSUES.md](MASTER_ISSUES.md):
instants are `INTEGER` unix milliseconds, and the two redundant JSON blobs
(`profiles.settings_json`, `profile_tracks.metadata_override_json`) and the
redundant `track_features.scale` are not created.

## M3 — what was actually built

Profiles work end to end: created, renamed, switched, deleted, and still there
after a restart. 164 tests across the workspace.

- `infra/db/repositories/{profile_repo,settings_repo}.rs` — the first two
  adapters. Rows are read into a plain `*Row` struct and validated into an
  entity separately, because the conversion can fail and rusqlite's row closure
  has no room for a domain error.
- `infra/system/{clock,paths}.rs` — the system clock, and the exact path layout
  of section 6. `BaseDirs` rather than `ProjectDirs`: the latter would have put
  the database in `%APPDATA%/Cadenza/config/`.
- `infra/events/event_bus.rs` — synchronous in-process fan-out. It snapshots the
  subscriber list and releases the lock before calling anything, so a handler
  that subscribes or publishes does not deadlock the bus.
- `core/application/services/profile_service.rs` — the use cases.
- `testkit/test_clock.rs` — a clock the test drives, starting at a fixed instant
  so nothing depends on when the suite runs.
- `app/src/{main,cli}.rs` — real wiring, plus a temporary command line so the
  milestone can be checked by hand. Both the CLI and its usage text say it is
  replaced in M6.

One port written in M1 was corrected: `SettingsRepositoryPort` took `&str
value_json`, which would have made `core` assemble JSON — serialisation is the
adapter's job. It now deals in `SettingValue`, and `serde_json` lives only in
`infra`. There were no implementations yet, so this was the last free moment to
fix it.

Deferred: `settings_service.rs` — the active profile is the profile service's
business and no per-profile setting has a consumer yet. `app/src/{wiring,config,
logging,lifecycle,runtime}.rs` — four dependencies do not need a wiring file.
`app/src/paths.rs` — it would duplicate `infra/src/system/paths.rs`; see
[MASTER_ISSUES.md](MASTER_ISSUES.md) finding 18.

Running the binary on a real machine found a defect the tests had not:
see finding 19.

## M4 — what was actually built

A folder of music becomes a library: scanned, tagged, hashed, de-duplicated,
with anything ambiguous held back for a decision. 197 tests.

- `infra/metadata/` — `lofty_reader` reads tags *and* stream properties in one
  pass, `normalize` cleans up what real tags contain, `artwork` caches cover art
  on disk and refuses anything that is not an image.
- `infra/library/` — `scanner` walks the filesystem in a stable sorted order,
  `hash` computes a blake3 content hash.
- Six more adapters: media files, tracks, artists, albums, genres, review queue.
- `core/application/services/library_service.rs` — the decisions: new, changed,
  unchanged, duplicate or problem.
- `testkit/audio_fixtures.rs` generates real WAV files, so duplicate detection
  can be tested with genuinely identical bytes.

`MetadataReaderPort::read` was changed to return format, stream properties and
tags together. Tags alone are not enough to fill `media_files`, and reading each
file twice to get the rest would have doubled the cost of every scan.

Running the binary found two defects the tests had not — see findings 21 and 22.
Both now have regression tests.

The filesystem watcher completes M4. `infra/library/watcher.rs` wraps `notify`
with a debouncer whose timing logic is pure and takes the instant as an
argument, so its tests assert on exact boundaries instead of sleeping. Copying
an album produces several events per file; acting on the first would mean
reading a file still being written.

`LibraryService` gained `apply_change`, which turns one filesystem change into a
library change, and `refresh_missing`, which reconciles what happened while
Cadenza was closed — a scan only ever meets files that exist, so on its own it
can never notice a deletion.

A file whose content matches a catalogued row whose file is gone is treated as
that file moved, not as a new one: it keeps its identifier, and with it its
listening history and playlist entries. See finding 23.

## M5 — what was actually built

Files become sound. All five required formats play through to the end with no
underruns, pause holds its position, and seek lands where it says it does. 244
tests.

- `infra/audio/ring_buffer.rs` — the single-producer, single-consumer ring the
  decoder and the audio callback meet across. Samples live in relaxed atomics
  rather than an `UnsafeCell`, so the realtime path contains no `unsafe` at all;
  on x86 a relaxed load is a plain move, and the price is a missed vectorisation
  in the copy.
- `infra/audio/symphonia_decoder.rs` — `DecoderPort::probe`, and `TrackStream`,
  the streaming half the engine drives directly.
- `infra/audio/resampler.rs` — rubato, engaged only when the file's rate differs
  from the device's. It usually does: Windows mixes at 48 kHz and most music is
  44.1 kHz.
- `infra/audio/stream.rs` — the shared state, the decode thread, and
  `fill_output`, which is the whole of what runs on the audio callback. Keeping
  it to one function is what makes section 8.2's contract checkable by reading.
- `infra/audio/engine.rs` — `CpalAudioEngine`. The cpal stream is not `Send` and
  `AudioEnginePort` is `Send + Sync`, so the stream lives on a thread that does
  nothing but hold it open.
- `testkit/temp_dir.rs` — a temporary directory for tests that need real files
  and no database.
- `app`: `cadenza play <file>`, with pause, seek and volume from the keyboard.
  Temporary, like the rest of the command line, and replaced in M6.

Running the binary found two more defects the tests had not — see findings 25 and
26. Both now have regression tests. Two additions to section 8.1's chain are
recorded as findings 27 and 24.

Deferred, each to the milestone that first has something to put in it:
`mixer.rs` and `crossfade.rs`/`gapless.rs` (M8 — a mixer over a single stream is
an abstraction with one implementation), `biquad.rs`/`eq.rs` (M9),
`visualizer.rs` (M10), and `clock.rs`, whose entire content is one atomic frame
counter inside `Shared`. `decoder.rs` alongside `symphonia_decoder.rs` would be a
second name for one adapter.

The engine itself is not covered by automated tests: it needs an output device,
and CI has none. Everything below it is — the ring, the resampler, the channel
map, the callback and the decode thread all run without one. What no test can
claim is that the result sounds right, and that is checked by running
`cadenza play`.

## M2 — deferred repository files

M2's task list says "repository stubs",
but sixteen files of `todo!()` are dead code that M3–M14 would rewrite. The
schema is proven by integration tests using plain SQL — which also means a
failure points at the schema rather than at a mapping layer. `profile_repo` and
`settings_repo` arrive in M3, where the plan already puts them.
