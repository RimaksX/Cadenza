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
| M6 | UI shell | done |
| M7 | Плейлисты, очередь, repeat/shuffle | done |
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

### The two questions that were open

Both were decided once the engine was working, and both are now closed.

**Per-profile genres** (finding 10). One listener correcting a genre used to
correct it for everyone sharing the file. Migration 13 adds
`profile_track_genres` and `profile_tracks.genres_overridden`; a profile sees its
own correction when it has made one and the file's own genres otherwise, and
"file this under nothing" is a decision distinct from having made none.
`GenreRepositoryPort` gained three methods, `LibraryService` gained
`genres_of` / `set_genres` / `reset_genres`, and `cadenza tracks` now numbers the
library and shows genres so `cadenza genre <n> ...` can be checked by hand.

**Section 16 of the master file** (finding 17) now says what is actually built
and points at the milestone that is really next. Nothing else in that file was
touched.

Running the two-profile demo found a third defect — see finding 28. The second
profile to import a shared file was getting its filename where the first got its
tags.

The engine itself is not covered by automated tests: it needs an output device,
and CI has none. Everything below it is — the ring, the resampler, the channel
map, the callback and the decode thread all run without one. What no test can
claim is that the result sounds right, and that is checked by running
`cadenza play`.

## Between M7 and M8 — input, menus and the interface audit

Pulled forward from M15 at the owner's decision: the three things they asked
for next — making a playlist, editing a track, adding tracks to the library —
all needed the same missing thing, and it was cheaper to build once than three
times. What that missing thing was: the interface had no text field, no menu
and no dialog.

Built: `Field`, `RowMenu` and `Dialog`/`PickDialog`, all from Slint's unstyled
primitives so they wear this design rather than the widget set's. With them, a
playlist is made, renamed and deleted from the window; a track is added to a
playlist or taken out of one, put in the queue or taken out of it, and removed
from the library; the library is searched; the queue is cleared; a playlist is
played from its tile or its page.

`scripts/audit_ui.py` came out of that work and stays. Five questions asked of
every line of markup, each one a defect that shipped first and was found by eye
— see `docs/TESTING.md`.

Not built, and known: editing a track's title, artist and album, which needs a
`LibraryService::edit_track` that does not exist yet — the menu deliberately
does not offer it. The settings screen. And adding a folder from the window,
which needs a native file dialog and therefore a decision about the stack that
is the owner's to make.

## M7 — what was actually built

Something to play next. The queue advances on its own when a track ends, goes
back, repeats in three modes, shuffles, and is still there after a restart.
Playlists are kept, reordered, played, and outlive the run that made them.
311 tests.

- `core/domain/queue.rs` — the order rules, and only them: manual entries
  outrank the continuation, repeat one holds the track, repeat all restarts the
  round from what has already played. The last of those works for a playlist or
  a radio batch as well as a library, because it rewinds through history rather
  than through any particular source.
- `core/domain/policies/shuffle_policy.rs` — a seeded permutation. Every track
  plays once before any plays twice, which is 9.2's first hard rule satisfied by
  construction; the scoring of 9.4 is M12's.
- `core/application/services/queue_service.rs` — what the order is made of, and
  the only caller of `PlaybackService::play_track`. Choosing a row queues the
  whole library, wrapping round to the tracks above it (finding 32).
- `core/application/services/playlist_service.rs` — create, rename, delete, add,
  remove, move, read. Entries are renumbered and rewritten in one transaction,
  which is what migration 6's deliberately non-unique index is for.
- `infra/db/migrations/m0014_queue.rs` — `queue_state` and `queue_entries`, the
  table section 7 never defines for a queue the requirements say must survive a
  restart (finding 31). One table for all four lanes; the lane is a column.
- `infra/db/repositories/{queue,playlist}_repo.rs`, and 23 integration tests
  across `queue.rs`, `queue_service.rs` and `playlists.rs` — real repositories,
  real schema, a fake device.
- `ui/` — the four transport buttons that were drawn and disabled since M6 now
  work; repeat is one button with three states. Queue, Playlists and one
  playlist are screens, reached from the sidebar. `components/TrackList.slint`
  is the page the library, the queue and one playlist all are, told different
  words. Playlists themselves are a grid of square tiles, following the albums
  grid of the reference — the square is where an uploaded cover will go
  (finding 36).
- One click plays a track and opens a playlist. The double click M6 chose is
  gone: nothing on screen can say a row needs two clicks (finding 35).
- `app` — `cadenza playlists` and seven `cadenza playlist` verbs. Creating a
  playlist needs a name typed in, and this interface has no field to type one
  into yet (finding 34).

Two things the interface does that the plan did not name, because the milestone
is incomplete without them. A row swaps its length for a queue button on hover,
the way it already swaps its number for a play button — without it the manual
queue is implemented, tested and unreachable. And the volume and position lines
became draggable, with the position reported only on release: a seek is a flush
handshake with the audio callback that can block for 200 ms, so reporting it
continuously would freeze the window for the length of the drag.

Smart playlists are not built. `is_smart` and `rule_json` exist in the schema
and no section of the master file says what a rule is; inventing one now would
mean designing it against no requirement. Finding 33 puts it in M12, where smart
shuffle has to express the same idea.

What rendering cannot check is still what it could not check in M6: a click, a
hover, a drag. The screens were rendered and read; the services behind every
control have tests; the playlist commands were run by hand against a real
database. The pointer itself remains the owner's to try.

## M6 — what was actually built

A window. It lists the library, plays what is double-clicked, pauses, stops,
seeks, changes level, and remembers which theme the listener prefers. 271 tests.

- `core/application/view_state.rs` — `PlayerView`, the whole of what the player
  bar draws. Listings are `domain::track::TrackSummary`, a read model rather than
  a view: the queue, playlists and radio will want the same rows and none of
  them is the interface.
- `core/application/services/playback_service.rs` — the only door between the
  interface and the audio engine, which section 4.3 forbids the UI to open
  itself. Eight tests against a fake engine cover what the buttons do.
- `TrackRepositoryPort` gained two projections and `infra` one `LEFT JOIN`. A
  listing needs names where an edit needs identifiers, and resolving them row by
  row would turn one query into thousands.
- `ui/slint/` — `theme/{tokens,dark,light}`, `components/{AppShell,Sidebar,
  PlayerBar,TrackRow}`, `views/LibraryView`, `app_window`. No component names a
  colour or a size of its own.
- `ui/src/` — `view_models/` (formatting, tested without a window), `controller`
  (translation and calls, no rules), `app` (window, callbacks, a 250 ms tick that
  asks the engine where it is).
- `app` — `cadenza` with no arguments now opens the window; every verb still
  works, and `cadenza status` is the old default under its own name.

The visual language follows the reference mockup rather than its markup: one warm
hue climbed in lightness so depth comes from a step rather than a shadow, a serif
for names, mono capitals for anything read as a number, hairlines instead of
boxes. Placeholder system faces until the real ones are bundled.

The sidebar ends with the listener: a rule across its full width, an initial in
a disc, the name, and a caption. It is the one static thing in a menu otherwise
made of places to go.

A theme switch briefly lived in that section and was taken out again — a
preference is not an identity, and it belongs on the settings screen with the
other preferences. Until that screen exists the palette is set with
`cadenza theme <dark|light>` and read from the profile at startup, so both
themes work as section 2.10 requires.

Worth keeping from building it, because the same trap waits on the settings
screen: a segmented control's halves must fill their well *exactly*. The well
was 28 high with 2 of padding around halves 22 high, which left two pixels for
the layout to place — all of them below the highlight, which is why it sat high
in its box.

The window draws its own frame. `no-frame` on the Slint window and a `TitleBar`
as the shell's first row: the name and the current section on the left, the
three controls on the right where this platform puts them — the reference is
drawn as a Mac, and a window that argues with its own operating system is worse
than one that copies a mockup imperfectly. Dragging and double-click-to-maximise
are ours to implement; resizing is not, because Slint's backend keeps the edges
of an undecorated window live, cursors included.

It carries no title. The wordmark is already at the top of the sidebar and the
section is already the heading of the page below it; a third copy in a 40px
strip could only be set small enough to squint at.

Three traps, all found by looking at the window:

- `height: 100%` on a child inside a layout whose own height comes from its
  children is a circular constraint, and Slint resolves it by giving that row
  the whole window. The rest of the interface rendered as nothing at all.
- A disabled primary button must lose its fill as well as its glyph. Left
  filled, it shouts in the loudest ink in the palette while the icon inside says
  it cannot be pressed.
- A row must be tall enough for its title's *line box*, not merely for its font
  size. Instrument Serif asks for about 1.4 times the size; one step up in the
  type scale left the title 2px short of its line, and Slint did not clip it —
  it dropped the title from every row and left the rest of the columns in place.

Hovering a row turns its number into a play button — one click instead of two,
and the only thing on screen that says a row can be played at all. The double
click still works and stays the faster habit.

Deferred, each named rather than quietly skipped: the ~25 other views and
components section 5 lists, which need screens that do not exist yet;
`commands.rs` and `ui/src/commands/` (finding 30); `subscriptions.rs`, because
nothing else in this process changes the library while the window is open;
`scaling.rs`, which needs a settings screen; a custom title bar, because the
native frame costs nothing and looks like the platform.

The player bar follows the reference's three-part arrangement: what is playing
on the left at a fixed width, the transport and its progress line in the middle,
the level on the right at the same fixed width — so the transport is centred on
the window rather than on whatever is left over. The line stops short of both
edges, which is what leaves room for a title on one side and a slider on the
other. Play is the one inverted control on the bar: a filled circle in the
loudest ink with the glyph cut out of it.

There is no stop button. Pause holds the position and the file, which is what
stopping a song means to a listener; the service keeps `stop` for the queue to
use when it runs out in M7. Shuffle, repeat, previous and next are drawn and
disabled: all four need a queue to act on.

Running the binary showed a defect the tests could not: the column headers and
the rows disagreed by a few pixels, because a Slint layout gives each cell its
natural width first and shares only the remainder by stretch factor — so a
column was as wide as the longest string in it. Fixed by pinning the stretching
cells to `preferred-width: 0`.

What nothing here covers is the click itself. The window was launched and its
rendering checked; the listing is right, the theme applies, and the services
behind every button have tests. What was **not** verified is that a double-click
on a row arrives at `PlaybackService` — synthetic mouse messages are ignored by
the windowing backend, and driving the real pointer is not something to do on
somebody's desk. The first person to click is the check.

## M2 — deferred repository files

M2's task list says "repository stubs",
but sixteen files of `todo!()` are dead code that M3–M14 would rewrite. The
schema is proven by integration tests using plain SQL — which also means a
failure points at the schema rather than at a mapping layer. `profile_repo` and
`settings_repo` arrive in M3, where the plan already puts them.
