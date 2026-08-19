# Findings against PROJECT_MASTER.json

Contradictions, gaps and problem spots found while implementing, with the option
chosen for each. `PROJECT_MASTER.json` remains the source of truth; this file
records where it was silent, self-contradictory, or would have produced a bad
result if followed literally.

Anything marked **open** is not resolved in code yet and needs a decision.

---

## 1. The 50% and 10-second rules overlap on short tracks — resolved

Section 2.6 says a listen counts when more than 50% played, and counts as a skip
when switched away before 10 seconds. A 15-second interlude stopped at 9 seconds
satisfies both.

**Chosen:** completion wins. The listener heard 60% of it; the skip rule exists to
catch rejection, and treating every short track as a rejection would poison the
skip rate and radio's negative feedback with intros and interludes.

Implemented in `crates/core/src/domain/policies/history_policy.rs`, with the
conflicting case as a named test.

## 2. `play_events` can record an impossible outcome — resolved

Section 7.4 stores `completed INTEGER` and `skipped INTEGER` as two independent
flags. Nothing stops a row saying both, which the rules do not allow.

**Chosen:** the domain models one `PlayOutcome` enum — completed, skipped, or
partial. The repository maps it to the two columns on write and validates on read,
so the schema stays as specified while the impossible state becomes
unrepresentable in code.

## 3. There is no queue table — resolved (contract), open (schema)

Section 2.3 requires restoring the last queue, 2.5 makes the queue per-profile,
and section 5 lists `queue_repo.rs`. Section 7 defines no table for it, and the
migration list in section 11 has no migration that would create one.

**Chosen:** `QueueRepositoryPort` is declared now with `load` / `save` / `clear`.
The table is added in M7 alongside the queue service, which is when its shape is
actually known. Section 7 should gain that table when it does.

## 4. `profile_tracks` overrides metadata twice — resolved

Section 7.3 gives `profile_tracks` explicit `title`, `artist_id`, `album_id`,
`track_no`, `disc_no`, `year` columns *and* a `metadata_override_json` blob. Both
express the same thing, and two writable copies of one fact drift apart.

**Chosen:** the domain carries the explicit fields only. Recommend dropping
`metadata_override_json` from the initial migration in M2 — a JSON blob shadowing
typed columns cannot be queried, indexed or validated.

## 5. `track_features` stores the mode twice — resolved

Section 7.2 has `key INTEGER`, `scale TEXT` and `mode TEXT`. In practice "scale"
and "mode" name the same major/minor property.

**Chosen:** the domain has `MusicalKey { pitch_class, mode }`. Recommend keeping
`key` and `mode` in the migration and dropping `scale`.

## 6. Retention is both a constant and a column — resolved

Sections 1.4, 2.6 and rule 12.1 all state a fixed 30 days, but
`profiles.history_retention_days` implies it is configurable.

**Chosen:** the column stays and holds the real value, but 30 is a ceiling rather
than a default. `Profile::effective_retention_days` clamps, and
`retention_policy::cutoff` clamps again, so a hand-edited or corrupted row cannot
quietly extend how long personal data is kept. Shortening it is allowed.

## 7. The radio diversity term has the wrong sign — resolved (documented)

Section 10.4 writes the ranking as
`w1*mood + w2*transition + w3*preference + w4*freshness + w5*diversity_penalty +
w6*exploration_noise`. A penalty *added* with a positive weight rewards the
monotony it is supposed to discourage.

**Chosen:** documented in `radio_policy.rs`. When ranking is implemented in M13 the
diversity term subtracts. Section 10.4 should be corrected.

## 8. Radio ranking weights `w1..w6` are unspecified — deferred to M13

Unlike the transition score (9.3) and the similarity score (10.3), which give
exact weights, the final ranking leaves every weight as a symbol.

**Chosen:** not guessed. `radio_policy::similarity` implements the two specified
weight sets; ranking waits for M13 when it can be tuned against a real library.

## 9. `.m4a` cannot be mapped to a codec by extension — resolved

Section 2.2 lists AAC and ALAC as separate formats, but both live in `.m4a`
containers, so a scanner cannot fill `media_files.format` from the filename.

**Chosen:** `is_supported_extension` gates which files are worth opening;
`DecoderPort::probe` decides the actual format. The two concerns are separate
functions rather than one lookup table.

## 10. Genre edits leak between profiles — resolved

Section 2.1 lets a profile edit metadata locally, and 12.1 forbids profile data
leaking. But `track_genres` in section 7.2 is keyed on `media_file_id` alone, so
one profile correcting a genre changed it for every profile sharing that file.

Three options were open, none free:

1. Add a per-profile genre override table. Correct, costs a table and a join on
   every genre query.
2. Accept it: genre is a property of the recording, not of the listener. Cheapest,
   but contradicts 2.1 as written.
3. Keep the tag-derived genre global and add profile-scoped *tags* separately.
   Most flexible, most work.

**Chosen: option 1.** It is the only one that honours both rules as written, and
it matches what `profile_tracks` already does for title, artist, album and year —
genre was the odd one out only because it is many-to-many and so could not be a
column. Option 2 would need 2.1 amended; option 3 invents a second vocabulary the
interface would then have to explain.

Implemented in migration 13:

- `profile_track_genres (profile_id, media_file_id, genre_id)`, keyed to the
  library entry so that deleting a profile takes its corrections with it, and a
  track removed and re-added keeps them.
- `profile_tracks.genres_overridden`, because "file this under nothing" is a
  decision a listener can make, and an empty override is otherwise
  indistinguishable from having made none.

`track_genres` stays as the tags describe the recording. A profile sees its own
correction when it has one and the file's own genres otherwise. The leak itself
has a test: two profiles, one file, different genres.

## 11. Monthly statistics against a 30-day window — not a contradiction

Section 2.6 asks for listening by month while retention is 30 days. The same
section says "в пределах доступной истории", which already concedes the point: a
monthly view shows a partial month. Recorded here so it is not re-litigated.

## 12. Two entities had no home in section 5 — resolved

`analysis_jobs` (section 7.1) and `AnalysisJobRepositoryPort` (section 4.4) both
exist, but section 5 lists no file for the entity, and `ClockPort`'s return type
has no listed home either.

**Chosen:** added `domain/analysis.rs` and `domain/value_objects/timestamp.rs`.
Section 5 should list both.

## 13. Symphonia's AAC and ALAC coverage — checked at M5, adequate

Section 2.2 requires AAC and ALAC; section 3.4 fixes Symphonia as the decoder.
Symphonia's AAC support is limited to AAC-LC, and gapless MP3 needs LAME/Xing
delay and padding tags to be read correctly.

**Verified at M5** against real files in all five formats: MP3, AAC in MP4, ALAC
in MP4, FLAC and WAV all probe correctly and play through to the end with no
underruns. No alternative decoder adapter is needed and the stack does not
change.

Two limits stand, neither of them blocking:

- HE-AAC is not decoded. A container does not say which profile it holds until it
  is opened, so `supports` answers per format and `probe` is the real check.
- The MP3 delay and padding tags are read by nothing yet. They matter for gapless
  joins rather than for playback, and belong with M8.

## 15. Timestamps are `TEXT` in some columns and `INTEGER` in others — resolved

Section 7 stores `media_files.file_mtime` as `INTEGER` and `media_files.created_at`
as `TEXT`, in the same table. Both are instants.

**Chosen:** every instant is `INTEGER` unix milliseconds. It matches the domain's
`Timestamp` exactly, sorts and indexes correctly, needs no calendar library in the
infrastructure layer, and removes a format-parsing step from every read and write.
`TEXT` would also only compare correctly if every writer used an identical,
zero-padded UTC format forever.

`daily_*.date` stays `TEXT`: a civil date in the listener's timezone is genuinely
a different thing from a point in time, and `YYYY-MM-DD` sorts and groups
correctly as text.

## 16. `profiles.settings_json` duplicates the `profile_settings` table — resolved

Section 7.1 gives `profiles` a `settings_json` blob while section 7.7 defines a
`profile_settings` key/value table. Same reasoning as finding 4: two writable
copies of one setting drift apart, and a blob cannot be queried or constrained.

**Chosen:** `settings_json` is not created. `profile_settings` is the only place a
per-profile setting lives.

## 18. Data paths have two homes in section 5 — resolved

Section 5 lists both `crates/app/src/paths.rs` and
`crates/infra/src/system/paths.rs`. Resolving `%APPDATA%` is an operating-system
call, and section 4.2 puts operating-system calls in infrastructure.

**Chosen:** only `infra/src/system/paths.rs` exists. `app` calls it. A second
copy in `app` would be either dead code or a second answer to "where is the
database", which is the sort of question that must have exactly one.

## 19. The migration runner trusted a foreign bookkeeping table — fixed

Not a contradiction in the master file but a defect in the M2 code, found by
running the binary rather than the tests.

`%APPDATA%/Cadenza/app.db` on the development machine held a database from an
earlier, abandoned build with a different schema — `created_at TEXT`,
`track_features.scale`, and a `schema_migrations` table carrying versions 1 to 5
but no `name` column. The runner read those versions as its own, skipped the
five migrations they named, and failed on the sixth with "table
schema_migrations has no column named name" — five migrations away from the
actual problem.

No data was lost: each migration runs inside a transaction with the row that
records it, so the failed one rolled back and the file was not modified.

**Fixed:** the runner checks the shape of `schema_migrations` before believing
anything in it, and refuses a file it did not write with a message that says so.
Covered by a regression test that reproduces the exact table.

The lesson is worth keeping: a green test suite said nothing about what happens
to a file the tests never create. Every milestone from here runs the binary as
well as the tests.

## 20. Import orchestration is placed in infrastructure — resolved

Section 5 lists `infra/src/library/importer.rs`, `duplicate.rs` and `review.rs`.
Deciding whether a file is new, changed, a duplicate or a problem is a business
rule, and section 4.1 keeps business rules out of the layer that touches disks.

**Chosen:** `infra/library/` holds mechanisms only — walking directories and
hashing. The decisions are in `core/application/services/library_service.rs`.
`duplicate.rs` would have duplicated `core/policies/duplicate_policy.rs`, and
`review.rs` is the review repository. Three files not created.

Section 5 also gives each of artists, albums and genres its own repository file.
They are a dozen lines each over the same three-table corner of the schema and
are always changed together, so they share `catalog_repo.rs`.

## 21. A file awaiting a decision was imported by the next scan — fixed

Found by running the binary, not by the tests.

A duplicate is catalogued but deliberately kept out of the library until the
listener decides. On the next scan that file took the "nothing changed" fast
path, which found no library row and helpfully created one — importing the very
file the review queue was holding back. The queue looked correct the whole time;
only the library was wrong.

**Fixed:** the fast path checks for an unresolved review entry before adding
anything. Regression test included.

## 22. Duplicates were detected against files that no longer exist — fixed

Also found by running the binary.

The catalogue is global and outlives the profiles that used it, so it fills up
with rows for files that have since been deleted or moved. A new file whose
contents matched one of those rows was held back as a duplicate of something the
listener could no longer look at — leaving them an empty library and a decision
they could not act on.

**Fixed:** a duplicate only counts if the other copy is still on disk. A row that
points at nothing is marked `file_state = 'missing'` on the way past, which is
what that column is for. Regression test included.

The sweep that marks *every* vanished file missing, rather than only the ones a
duplicate check walks past, belongs with the filesystem watcher.

## 23. A rename is two events, and became two library entries — fixed

Found by running the watcher, not by the tests.

Windows reports a rename as a removal followed by a creation, in either order.
Treating the creation as a new file left a phantom entry pointing at nothing and
a second one beside it, and the moved file lost its identifier — and with it the
listening history and playlist entries hanging off that identifier.

**Fixed:** content matching a catalogued row whose file is gone is that file in a
new place. The row moves; nothing is created. Order does not matter, because the
test is whether the old path still exists rather than which event arrived first.
Covered by a test that feeds the two halves in separately.

A note for whoever debugs a scan next: the catalogue is global and outlives
profiles, so deleting a profile does not clear it. Two experiments using the same
folder path will contaminate each other, and the second will look like a bug in
duplicate detection. It cost an hour here.

## 24. Two components decide what format a file is — resolved

Finding 9 chose `DecoderPort::probe` as the thing that decides an `.m4a` is AAC
or ALAC. M4 then shipped the scanner filling `media_files.format` from lofty,
because reading tags, stream properties and format in one pass is one file open
instead of two across a five-thousand-track library. M5 adds a second opinion.

**Chosen:** they answer different questions and both stay.

- lofty is the catalogue's oracle. It is what a scan can afford.
- Symphonia is playback's oracle, and it is the authoritative one: it is the
  component that has to produce samples.

A disagreement is not a tie to break. It means the file is catalogued as
something Cadenza cannot play, which is what `file_state = 'error'` exists for.
Nothing writes that yet — playback failure has no path back into the library
until there is a player in M6 to notice it — and this is the note that says so.

Symphonia also reports no average bitrate, so `properties.bitrate` from a probe
is always `None`. lofty fills that column; nothing on the playback path reads it.

## 25. Every track began with an underrun — fixed

Not a contradiction in the master file but a defect in the M5 code, found by
running the binary rather than by the tests.

cpal starts asking for samples the moment the stream is created, and the decode
thread is at that point still opening the file. The callback found an empty ring
and reported an underrun on every track — one for WAV and AAC, four for FLAC,
whose first packet takes longest.

Nothing was audible: the callback outputs silence when the ring is dry. But the
underrun counter is the number the next person will look at when playback
stutters for a real reason, and a counter that is never zero says nothing.

**Fixed:** the callback does not start consuming until 50 ms is queued, and the
gate is cleared on every flush so a seek refills before it plays instead of
starting on whatever fragment arrived first. Underruns are now zero across all
five formats. Covered by a test.

## 26. The end of every track was unseekable — fixed

Also found by running the binary, four seconds into a six-second file.

The decoder runs up to two seconds ahead of the speakers, so it reaches the end
of a file long before the last note is heard — and it dropped its `TrackStream`
there. Seeking after that point answered "nothing is loaded to seek in", while
the track was still playing and its progress bar still moving. On a file shorter
than the prebuffer it was unseekable from the first moment.

**Fixed:** reaching the end of the stream marks the track finished and stops
decoding, but the file stays open until the track is stopped or replaced. Two
tests cover it: a finished track can still be seeked back into, and an unloaded
one cannot.

This is the fourth defect in a row that the tests were green through and running
the binary caught. The tests never had a file play *past* the decoder.

## 27. The signal chain has no channel mapping stage — resolved

Section 8.1 runs decoder, resampler, EQ, volume, mixer, tap, output. It accounts
for a file's sample rate differing from the device's and not for its channel
count differing, which happens just as often: a mono recording on a stereo
device, or either on a machine reporting a 5.1 layout.

**Chosen:** a channel map between decoding and resampling, so the resampler and
everything after it work at one fixed layout. Mono is duplicated across the
outputs — sending it to the left channel alone is indistinguishable from a broken
speaker — and stereo into one output is averaged rather than halved.

Wider layouts are truncated to the first channels rather than folded down, which
loses the centre of a 5.1 file. Proper downmix coefficients are a table and a
listening test, and the material section 2.2 describes is overwhelmingly stereo.
Section 8.1 should gain the stage.

## 28. The second profile to import a file got its filename, not its tags — fixed

Found by running the binary while checking finding 10, and not by the tests.

Two profiles sharing a folder: the first scan reads tags and fills the library
properly. The second profile's scan finds every file already catalogued and
unchanged, takes the fast path that never opens a file, and added its library
rows from the filename — `mysterons` instead of `Mysterons`, with no artist and
no album. Every listener after the first got a worse library than the first one,
for no reason they could see.

**Fixed:** joining a file that is new to *this* profile reads its tags, which is
the same cost the first profile paid and is paid once per listener per file. The
early return for a track already in the library is untouched, so a rescan of an
unchanged library still opens nothing.

Copying the other profile's row would have been cheaper and wrong: it would hand
over their corrections, which is the leak 12.1 forbids and finding 10 fixes.

The fix also deleted the duplicated `Track` construction — the fast path now
delegates to the same `upsert_track` the ordinary import uses, which is why the
two could disagree in the first place.

## 29. `forbid(unsafe_code)` cannot survive generated code — resolved

Every crate except `infra` opens with `#![forbid(unsafe_code)]`. Slint compiles
the markup into Rust that carries its own `allow(unsafe_code)` for the vtables
it builds, and `forbid` outranks any inner `allow` — including one nobody wrote
by hand. The UI crate would not compile.

**Chosen:** `#![deny(unsafe_code)]` in `crates/ui` and nowhere else. Hand-written
code in that crate is still refused an `unsafe` block; what changes is that
machine output may exempt itself, which is the case the lint was never aimed at.
`core`, `app` and `testkit` keep `forbid`.

Worth knowing rather than hiding: the realtime audio path, which is the place
where `unsafe` would actually be tempting, is in `infra` and contains none — see
`audio/ring_buffer.rs`.

## 30. The UI's command channel is the services themselves — resolved

`docs/UI_CONTRACT.md` and section 5 both describe commands travelling up from
the interface through `core/src/application/commands.rs`, and section 5 lists a
`crates/ui/src/commands/` directory of eight files beside it.

**Chosen:** the application services *are* the command surface.
`PlaybackService::toggle` is a command; wrapping it in a `Command::TogglePlay`
enum that a dispatcher immediately matches back into the same call adds a name,
a match arm and a file per verb, and removes nothing.

The contract it was there to protect is unchanged and still checkable by
reading: `crates/ui` depends on `cadenza-core` and `slint` and on nothing else,
so a view cannot reach a database, a file or the audio engine whatever it calls.

An enum earns its place when a command needs to be queued, retried, logged or
undone — a background scan that must not block the window, which is M7's
problem. Section 5's `commands/` directory and `commands.rs` are not created
until then.

## 17. Section 16 status is stale — resolved

`16_Текущий_статус` read "Реализация кода еще не начата" and `next_step: M0`
through five completed milestones.

Keeping it current is maintenance rather than an architecture change, but the
master file is the source of truth and should not be edited as a side effect of
implementation work, so it waited for the go-ahead.

**Chosen:** `status` now names what is finished and points at this file and
`PROJECT_PLAN.md` for the detail; `next_step` is the milestone actually next.
Nothing else in the master file was touched — `implementation_order` and every
other section stand as written, and the deviations recorded here stay recorded
here rather than being edited into the source of truth.

## 31. The queue has to survive a restart and has no table — resolved

Section 2.3 lists "восстановление последней очереди" among the playback
features and 2.5 gives every profile "собственную очередь", but section 7
defines no table for either. `QueueRepositoryPort` was declared in M1 with a
note saying the storage shape was M7's to settle. This is M7.

**Chosen:** `queue_state` (one row per profile: repeat mode, shuffle, when it
was written) and `queue_entries` (one row per queued track), in migration 14.

One table for all four lanes rather than four tables, with a `lane` column
holding `current`, `manual`, `upcoming` or `history`: they store the same thing
in a different role, and the role is one column. `current` is a lane with at
most one row rather than a column on `queue_state`, so that a lane change is a
lane change and not a move between tables.

Saving replaces every row for the profile inside one transaction. A queue is a
few dozen rows, it changes as a whole every time a track starts, and a diff that
gets one lane wrong is a queue that silently plays the wrong thing.

`queue_entries.media_file_id` cascades from `media_files`: a queued track whose
file has left the catalogue has nothing left to play.

## 32. "Repeat All начинает заново" does not say what the list is — resolved

Section 2.3 says repeat all restarts the list after its end, without saying
whether "the list" is the queue or the library — and choosing a row halfway
down a library makes the two differ.

**Chosen:** choosing a row queues the whole library, wrapping. The continuation
is everything after the row followed by everything before it, so a round covers
every track exactly once and ends where it began. Repeat all then means what the
section says — the same round again — and a listener who starts halfway down
still hears the first half.

The domain does the restarting from `history` rather than from the library
(`Queue::advance`), so the rule holds for a playlist or a radio batch too, where
there is no library to fall back on.

## 33. Smart playlists have a column but no language — deferred

`playlists.is_smart` and `playlists.rule_json` exist in section 7.3, and M7's
task list says "playlist CRUD" without distinguishing curated from smart. No
section of the master file defines what a rule looks like: not its operators,
not its fields, not how it combines with the library filters of 2.1.

**Chosen:** M7 implements curated playlists only. `is_smart` is written as 0 and
`rule_json` as null, and the schema's `CHECK (is_smart = 0 OR rule_json IS NOT
NULL)` keeps a half-defined smart playlist from existing at all.

Inventing a rule language now would mean designing it against no requirement and
migrating it later when one arrives. The natural home is M12, where smart
shuffle already has to express "tracks like these" — the same selection problem,
and the two should share one vocabulary rather than grow two.

`Playlist::is_manually_ordered` already gates reordering on it, so the day a
smart playlist can exist, the code that must not reorder it already refuses.

## 34. Making a playlist needs a name typed in — CLI for now

Playlists are created, renamed and deleted from the command line
(`cadenza playlist new <name>`), not from the window. The interface plays them,
reorders nothing and creates nothing.

**Why:** every one of those actions needs a text field, and this interface has
none — no input control, no dialog, no context menu. Adding the first one is a
design decision about how Cadenza asks for a word, and it belongs with the rest
of the interface design rather than smuggled in as a side effect of playlists.

This is the same split the library already lives with: folders are added and
scanned from the command line (M4) and browsed in the window (M6). The window
does what you do while listening; the command line does what you do to set up.

The interface reaches everything a listener does *while playing*: play a
playlist, queue a track, reorder nothing. The rest waits for M15.

## 35. A double click is not discoverable — replaced by one

Rows opened and played on a double click, which is the file-manager habit and
was chosen in M6 to leave the single click free for a selection.

**Chosen:** one click plays a track and one click opens a playlist. Nothing on
screen can say "this takes two clicks", and a listener who tries once and gets
nothing concludes the row is not clickable — which is what happened.

What that spends is the single click a selection would have wanted. Selection
can have the modifier keys and a menu, both of which are teachable; playing
cannot be taught to anybody at all. Nothing here is destructive, so a click that
starts a track costs a wrong track and a second click to fix it.

## 36. Playlists are tiles, and the tile is where a cover goes — column pending

The index was a list of names. It is a grid of square tiles now, following the
albums grid of the reference: the square carries the name set large, its corner
marks, and whose list it is; the caption under it repeats the name and says how
much is in it.

**Why the square exists before there is a picture:** the owner asked for covers
a listener can upload. The tile is the shape that holds one, so the page will not
change layout the day the first cover arrives — the square stops drawing a name
and draws an image instead.

**What that still needs**, none of which is built:

- a column. `playlists` has no cover field in section 7.3; a migration adds
  `cover_path TEXT`, holding a path inside the artwork cache rather than the
  bytes, the way `FileArtworkCache` already keeps embedded art.
- a way to choose a file, which means a native file dialog — the first one in
  Cadenza. `LocalFileSystem` can copy it into the cache; nothing can currently
  ask a listener which file.
- a decision about what happens to a cover when the playlist is deleted.

Recorded here rather than half-built: a column nothing writes is a column that
has to be migrated twice.

## 37. The queue's rows acted on the library — fixed

Every row in the queue offered to add itself to the queue, and clicking one
called `play_from_library`, which rebuilds the queue out of the library and
throws away what was waiting. Both are the same mistake: the shared listing
component knew what a row was, but not which list it was in.

**Chosen:** `TrackList` carries a `queued` flag, and the queue sets it. A queued
row removes instead of adding, with the trash icon and REMOVE where QUEUE would
be, and a click jumps to that entry keeping the rest of the queue
(`QueueService::play_at`).

Both name the entry **by its position in the listing**, not by its track. The
same track may legitimately be waiting twice — the manual queue exists so that
it can — and an identifier cannot tell the two copies apart. The service counts
positions the same way the listing was drawn, skipping entries whose file has
left the library, so the row that answers is the row that was pointed at.

What jumping forward does with what it skipped: those entries become history,
because that is what "previous" walks back through.

## 38. The mixer stands before the ring, not on the audio thread

Section 8.1 gives the chain as `... -> stream volume / fade -> Mixer ->
visualizer tap -> audio output`, and ADR 5 was written expecting to build the
mixer where a mixer usually is: on the realtime callback, pulling from two
rings. Section 8.2 then forbids that callback almost everything.

**Chosen:** the two decoded streams are mixed on the **decode** thread, before
the ring, and the callback stays what it was — pop, ramp, multiply, count.

The order of the chain is unchanged; only the side of the ring it sits on is.
What that buys:

- **The join is sample-accurate by construction.** The thread that mixes is the
  thread that produced both sets of samples, so the frame the fade begins on is
  chosen, not raced for.
- **No new realtime code beyond eight lines.** The callback gained one branch —
  crossing the mark the decoder left — and it is arithmetic on atomics.
- **The whole feature is testable without a device.** `stream.rs` decodes two
  real files and asserts on the samples that would have gone to the speakers.

What it costs: the transition point is fixed once the samples are queued. A seek
into the last seconds of a track throws away a fade that had already begun, so
the engine drops what it had armed and the queue arms it again — which it does
on every tick anyway, by asking `armed()` rather than by remembering.

The mixer moves onto the callback the day two streams have to be started
independently of each other. Nothing in sections 2.3, 2.4 or 8 asks for that.

## 39. Preloading defaulted to off under a comment saying it is always on

`PlaybackSettings` derived `Default`, which makes every flag false —
`preload_next` included, directly under a doc comment calling it "effectively
always on". Nothing read the struct until M8, so nothing had noticed.

**Chosen:** `Default` is written out. Preloading on, crossfade off, four
seconds when it is turned on — which is what 2.4 says and what the doc comment
already claimed. A derived default that contradicts its own documentation is a
bug waiting for its first reader.

## 40. `gapless.rs` is not written, and will not be

Section 5 lists both `crates/infra/src/audio/crossfade.rs` and
`.../gapless.rs`. M8 built the first and left the second out.

**Chosen:** a gapless join is the *absence* of processing — the next lane's
samples are simply the next samples pushed, on the frame the last one ran out.
There is no length to decide, no curve to apply and no state to keep. A file
named for it would hold a comment.

`crossfade.rs` exists because a crossfade does have arithmetic of its own: how
long the fade runs given what is left of the outgoing track, and the two gains
at each point along it. That is worth reading and testing apart from the
decoding it is applied to; nothing else about transitions is.

## 41. The advanced equaliser is parametric, not a ten-band graphic

Section 8.5 specifies the advanced mode as ten fixed bands at 31, 62, 125,
250, 500, 1k, 2k, 4k, 8k and 16k Hz, with a gain each. `eq_policy` enforced
exactly that, down to rejecting a preset whose centre frequency had moved.

The owner asked for the shape NothingX uses instead, having listened to it:
eight bands, each with its **own frequency, Q and gain**. That is a different
feature rather than a different drawing of the same one, so it was put to them
as a change to the master with the alternatives beside it, and they chose it.

**Chosen:** `EqMode::Advanced` becomes an eight-band parametric equaliser.
Every band is a bell — which is what parametric means — and the shelves stay
where they were always more useful, on the simple mode's bass and treble.

What that costs and what it does not:

- **No data migration.** `eq_presets.advanced_bands_json` has never been
  written: the table has had no adapter since it was created in M2. Only the
  shape of what goes into it changes, and there is nothing in it to change.
- `validate_advanced_bands` stops checking centre frequencies against a fixed
  list — there is no list any more — and starts checking that each band's
  frequency, Q and gain are inside the ranges a filter can be built from. That
  check matters more now, not less: these numbers reach the realtime filter,
  and a Q of zero there is a divide by zero in the coefficients.
- Bands are **not** required to ascend. A parametric equaliser is a set of
  bells, not a row of sliders; the screen sorts them to draw the curve, and
  refusing to save an out-of-order set would be a rule with no purpose behind
  it.
- The nine built-in presets of 2.8 are unaffected in name and become more
  precise in content: a curve that had to be approximated across ten fixed
  points can now say where it wants its bell.

What is lost, stated plainly: a graphic equaliser is legible at a glance and a
parametric one is not. Nobody has to meet it — the presets are one click on the
same screen — but the listener who opens the advanced mode now needs to know
what Q means.

## 42. Nothing in the plan builds a settings screen

PROJECT_MASTER asks for per-profile theme (2.10, 7.1), crossfade with a length
(2.4), history with a retention window (2.6) and library folders (2.1) — and
none of the sixteen milestones builds anywhere to change them. M15 is the
closest and is about integrating a design, not adding a screen. Every one of
these settings has therefore lived on the command line since the milestone
that introduced it, which is a player that has to be operated from a terminal.

**Chosen:** the settings screen is built as an interlude before M11, the way
the input layer was built between M7 and M8, and this is recorded rather than
folded into a milestone that did not ask for it.

It holds the four things the specification already stores: the theme, the
crossfade and its length, whether history is kept, and where the music is.

## 43. Choosing a folder needs a dialog, and a dialog needs a dependency

The library has taken folders since M4 and had no way to be given one except
`cadenza add-folder <path>`. The owner asked for that to be solved properly and
offered an alternative: an application folder created at install time, with the
listener putting music into it.

**Chosen: both, and in that order of importance.**

- **The chooser is the way in.** Cadenza's premise is that the music is already
  the listener's and already somewhere — a second drive, a network share.
  Making our folder the only one it can be in means asking them to move a
  library to satisfy a player, which is the player serving itself.
- **The suggestion answers the empty case.** Somebody with nothing should not
  have to invent a place. The screen offers the system's own music folder with
  a room of ours inside it — `…/Music/Cadenza` — and **creates it on a press,
  never on a first run**. Writing into somebody's filesystem while nobody is
  looking is a thing that has to be forgiven afterwards.

The dependency is `rfd`, and it is worth being precise about why: reaching
`IFileDialog` directly is COM, and COM is `unsafe`, which every crate here
forbids. `rfd` is a thin safe wrapper over that one call, taken with default
features off — the defaults are a Linux portal and an async runtime, and
Windows is the whole target (1.2).

What it does not do: parent itself to our window. The window lives in
`cadenza-ui`, which cannot see `cadenza-infra` and must not. The dialog still
opens in front, because the process asking is the foreground one; what it loses
is being *owned* by the window, which shows only if somebody clicks behind it.
A handle can be threaded through the port the day that matters.

## 44. A track taken out of the library had no way back

Removing a track is a decision and a scan must not undo it — a file sitting in
a watched folder would come back every few minutes, which makes the row menu's
"Remove from library" meaningless. So `import_file` takes the fast path for an
unchanged file and leaves the removal alone.

The consequence was worse than the bug it avoided. The file is on disk, in a
watched folder, catalogued and unchanged, so **every** later scan takes that
same fast path: removing the folder and adding it back does nothing, deleting
the file and restoring it does nothing, and the only way back is an `UPDATE`
against the database. The owner met this directly — five tracks, a fresh
folder, an empty library and a scan that cheerfully reported five files
unchanged.

**Chosen:** a routine scan still leaves removals alone; **adopting a folder
brings back what is inside it.** `LibraryService::adopt_folder` is what
choosing a folder runs — from the window and from `cadenza add-folder`, which
now scans as part of adding rather than telling the listener to run a second
command.

The rule in one line: *pointing at a folder is a statement about everything in
it, and it is newer than an older statement about one file.*

## 45. The queue filled itself, and emptying it stopped the music

Playing a row from the library put the whole rest of the library into the queue
behind it. It made "next" mean something, and it made everything else mean
nothing: the queue showed hundreds of tracks nobody queued, clearing it stopped
playback outright — the current track was one of its entries — and shuffle
reordered that whole pile, so the two or three tracks a listener had actually
asked for were lost in it.

PROJECT_MASTER 2.3 describes a manual queue and an automatic continuation as
two lanes and never says the continuation must be *stored*. Storing it was the
mistake. The owner stated the rule plainly: the queue is filled by a person,
and an empty queue means the library simply plays on.

**Chosen:**

- **Nothing is queued by playing a track.** `play_from_library` starts the
  track and leaves both lanes as they were, so anything queued by hand still
  plays first.
- **What follows a library track is worked out, not remembered.**
  `playback_policy::next_in_library` answers it from the library listing: the
  next row in order, or — under shuffle — a track that has not had its turn
  yet, which is 9.2's first hard rule enforced against the play history instead
  of against a materialised pool.
- **Clearing empties the lanes and touches nothing else.** The track that is
  playing left the queue when it started; clearing is a statement about the
  list, not about the music.
- **Shuffle is about what comes after what was asked for.** The manual queue is
  never reordered — those tracks were put in an order by hand. A playlist's
  remaining tracks still are: they are the continuation, and that is the thing
  shuffle governs.

Two consequences worth naming. Repeat off now **stops at the bottom of the
library** instead of wrapping round to the top, which is what "stop once the
queue is exhausted" has to mean when the queue is a library — repeat all still
wraps. And `has_next` says yes whenever something is playing: knowing better
means listing the library, and that view is rebuilt four times a second.

The queue is now the only place in Cadenza that answers *what did you ask for*,
and the library answers *what comes after*. They are different questions and
they were being kept in the same list.

### What was armed had to be able to change

Making the queue a short human list exposed an older fault. The engine is
handed the following track seconds before it is needed, and the queue then had
no idea *which* track that was — so a track queued at the last moment was shown
as playing while the engine played the one it had already decoded.

The service now remembers what it armed and re-arms whenever what follows is no
longer that. The cost is a second decode setup on a queue edit, and one place
where it is imperfect: an edit made *during* a fade cannot recall audio already
mixed into the ring.

## 46. A track that ran out went on playing silence for ever

The end of the last track left the player showing it as playing, at a standstill
and in silence. Nothing advanced, nothing stopped, and pressing play did nothing
useful.

The queue was innocent. It watches for the one state only an ended track
produces — stopped, with a track still loaded — and that state never arrived,
because the decode thread went to sleep before it could be reported.
`Producer::is_idle` counted a *drained* lane as idle: the decoder stops several
ring-lengths before the listener does, so the moment the file was read to its
end the loop blocked on the next command. `produce` was never reached again,
and it is `produce` that calls `finish` and sets `ended`. The flag stayed false
for ever, so `state()` kept answering `Playing`.

It only ever showed with nothing armed behind the track: an armed lane makes
`is_idle` false, which is why every join, every crossfade and the whole of M8
worked. Before the queue stopped filling itself (`MASTER_ISSUES` 45) the bottom
of the queue was the bottom of a whole rotated library and hardly ever reached.
Afterwards it is two tracks away, and the owner met it immediately.

**Chosen:** idle now means drained *and* ended. The loop naps its three
milliseconds a few more times, `produce` sets the flag, the ring plays out, and
`state()` reports stopped once it is empty — after which the thread blocks as
before and costs nothing.

Verified against a real device: `a_track_with_nothing_behind_it_reports_that_it_stopped`
in `tests/audio_join.rs` failed with `Playing` before the change and passes
after it.

The rule in one line: *a decoder that has finished is not a track that has
finished, and only the second one is worth telling anybody about.*

## 47. M11: the decisions analysis needed and the master did not make

Six things had to be settled to build DSP analysis. None of them contradicts
PROJECT_MASTER; all of them are absent from it.

**The table was already there.** `track_features` has existed since migration 3
— written when the catalogue was, because it describes a file rather than a
listener. M11 therefore adds no migration at all. (I wrote one first; the first
test run refused it, which is what an append-only schema with a real test is
for.) The `scale` column of 7.1 was settled back then too: it and `mode` name
the same major/minor property, and only `mode` is stored.

**Nothing could reach the table.** Section 4.4 lists `AnalysisJobRepositoryPort`
and no repository for the features themselves, so `TrackFeaturesRepositoryPort`
is new: get, save, and count for one extractor version.

**Which files still need analysing is asked in SQL.** A file needs it when it
carries no features from the running extractor *and* has not already spent its
attempts failing. The second half is what stops a broken file being picked up
for ever; the first is the whole of "повторный анализ не происходит".

**How much of a track.** Ninety seconds from the middle, mixed to mono at
22 050 Hz. Intros fade in and endings fade out, and neither describes the song.
Measured on real files: **151 ms each**, which is thirteen minutes of one core
for five thousand tracks — an hour at the background share. Nobody waits for it
and nobody is meant to notice it.

**How the CPU budget is kept.** Not by thread priority. `SetThreadPriority` is
a Win32 call and reaching it means `unsafe` or a dependency taken for one
function; `WindowsPriority` is therefore the no-op its port explicitly allows,
and the promise of 2.11 is kept by `analysis_policy` instead — the worker rests
four times as long as it works while music plays, twice as long when nothing
is. A share of the clock is a promise that does not depend on a scheduler
agreeing with it, and it can be tested, which a priority cannot.

**Valence is a heuristic and says so.** Tempo is a period and loudness is an
amplitude, but musical positivity is a judgement. What is computed is the
correlation a listener would recognise — major, brisk and bright feels happier
— weighted from the key, the tempo and the brightness. 12.1 forbids the trained
model that would do better, and an empty column would leave M13 treating every
track alike, so the honest answer is a documented approximation rather than
either.

Two smaller notes. `AnalysisKind::Metadata` and `::Hash` still have no
producer: M4 read tags and hashed files inline, and only `Features` is queued.
And autocorrelation cannot tell a tempo from half of it — a steady beat
correlates just as well with every second beat — so the choice is weighted by
a bell over the logarithm of the tempo, centred at 120. A real 70 or 170 still
wins on its own evidence; an artefact of doubling does not. The click-track
test caught this by reading 120 BPM as 60.

## 48. M12: what "smarter than random" is allowed to cost

The formula was already written down — 9.3's weighted sum has been in
`transition_policy` since M1 — so M12 is about the three decisions around it.

**Whose turn it is comes before what sounds best.** The round is what has not
been heard; the artist cooldown vetoes inside it; only then does the score
decide. A rule that the score could overrule would not be a rule, and 9.2 calls
these hard.

**The cooldown counts names, not identifiers.** `artist_on_cooldown` took an
`ArtistId`, which nothing in a listing carries: `TrackSummary` has a name
because a name is what gets drawn. Resolving thousands of rows to identifiers to
compare things a listener compares by ear would be work for its own sake. Two
different artists sharing a name are treated as one, which is also what the
listener would think.

**The best handful, not the best.** PROJECT_MASTER 9.4 says top 10–20 and a
weighted draw; both halves matter. Always playing the closest match turns five
thousand tracks into forty, and 9.1 asks in the same breath for shuffle to keep
feeling like shuffle. With features that differ sharply the best transition wins
about seven draws in ten — which is the number the test asserts around, because
asserting it wins *every* time would be asserting the opposite of the
requirement.

Two consequences worth stating. An unanalysed track scores neutral rather than
zero, so a half-analysed library plays all of itself instead of only the half it
knows about — which matters because analysis is deliberately slow. And the
features are read once per track change and indexed by identifier: the obvious
loop is a scan per candidate, which on five thousand tracks is twenty-five
million comparisons to answer one question.

`playback_policy::next_in_library` is now order only. What shuffle plays next
has different inputs and a different rule, and one function pretending to answer
both took a `shuffle: bool` and did two unrelated things underneath it.

## 49. M13: what radio measures when there is no history to measure

The ranking of 10.4 has a freshness term, and freshness means "how long since
you heard this". Nothing writes listening history yet — `PlayEventRepositoryPort`
has no adapter, because recording and purging listens is M14's — so the term
had a name and no source.

Three ways out, and only one of them honest. A no-op adapter would make the
term silently constant while looking implemented. Dropping the term would mean
re-opening the formula later. What it does instead is measure **how long since
radio itself last offered the track**, which every station has recorded since
the first one: `radio_session_items.created_at`, grouped by file. For somebody
listening to radio that is the same question — a track offered an hour ago
should not come straight back — and when M14 starts writing history the source
can widen without the formula moving.

Two smaller decisions worth writing down.

**A verdict is about a pick, not a tally.** Pressing dislike twice on the same
offered track says one thing twice; the row is updated, not appended, and the
weight that reaches the ranking is one dislike. Accumulation happens across
*offers* — a track disliked in three different sessions is disliked three times
— which is the difference between an opinion and a click count. A test asserted
otherwise and was wrong.

**Feedback is not a ban.** A disliked track keeps its place among the
candidates and simply scores the bottom of the preference term. Banning is what
the no-repeat rule does within a session; a preference that could ban would
make one irritated press permanent.

The batch is generated one pick at a time rather than by sorting once and
taking the top eight. Each pick becomes the *previous* track for the next
transition score, and joins the artist window the diversity term reads — so a
batch is a sequence that was reasoned about rather than a set that happened to
rank well.

## 50. "Stop the station" was a fourth way to do a thing with three ways already

The radio screen shipped with a STOP THE STATION button, and the owner asked
what it was for: a mood can be switched and a track can be played, so what does
stopping add? Nothing — and looking properly turned up two faults it had
introduced.

**It stopped the generation and not the music.** What the station had already
queued kept playing, so the button did not do what it said.

**A station that had ended was still being asked for more.** `refill_radio`
looked at the *queue* to decide whether radio was playing, and the queue still
held the picks. Once the lane ran low it called `next_batch` on a service with
no live session, which is an error — reported to the listener four times a
second for as long as those picks played.

**A skip on a hand-queued track was an error too.** With a station live, next
recorded a verdict against whatever was playing; if the station had never
offered that file, the update matched no row and failed. Skipping your own
choice is not a statement about the station.

**Chosen:** a station ends when something else plays. `play_from_library` and
`play_playlist` end it, which is what a listener means by choosing them; the
refill asks the service rather than the queue whether a station is live; and a
verdict about a track the station never offered is a no-op rather than a
failure. The button is gone, replaced by one line of text saying so.

The rule in one line: *a control that repeats what three other actions already
do is not a shortcut, it is a fourth thing to keep working.*

## 51. Three requirements no milestone was ever asked to build

Checking M0–M13 against their definitions of done, all fourteen pass. Checking
the same code against section 2 turns up three things the requirements ask for
and no milestone's task list mentions, so nothing has built them:

**Editing a track's title, artist or album in the interface** (2.1: "локальные
переопределения в БД"). The storage is ready and has been since M4 —
`profile_tracks` holds a per-profile title and artist, which is exactly the
local override the requirement describes — and genres can already be corrected
from the command line. What is missing is `LibraryService::edit_track` and a
way in.

**Showing a duplicate to the listener as a warning** (2.1). `import_review`,
its repository, its policy and `cadenza reviews` all exist; the four choices
the requirement lists — keep, add anyway, remove the existing one, edit the
metadata — have no screen to be offered on. A duplicate is currently detected,
recorded, and never mentioned to anybody using the window.

**Switching profiles in the interface** (2.5, whose three switching rules are
already implemented in `ProfileService`). The settings screen changes the
theme, the crossfade, the history switch and the folders of the profile that
is active; it cannot change which one that is.

None of the three is a defect: every one is a requirement with no milestone
behind it, in the same way the settings screen had none (`MASTER_ISSUES` 42).
M15 is the final *design* and M16 is packaging, so neither is a home for a
feature. They are recorded here to be scheduled deliberately rather than
discovered during packaging.

## 52. A page that had no widths, and a shell that was never given the window

The listening screen came up a third of the width of the window, its four
figures squeezed into a corner with their labels elided to "HE…" and "SK…", and
a band of bare window under the player bar. Two faults, and the first one I
"fixed" twice before understanding it.

**The page never said how wide its parts were.** Every other screen here states
the width of the things that have one — the settings screen has a 320-pixel
column and buttons at 100 and 148, a track row has a 240-pixel artist column
and lets the title take what is left. Those pages fill the window because their
parts add up to something, not because anything stretches them. The figures had
no width at all, so the page asked for the room four short numbers need and got
exactly that. A figure is a control; it is 176 pixels wide, which is what
"SKIPPED" under "2h 05m" needs, and the row of four now gives the page a width
worth opening at.

Before understanding that I tried `horizontal-stretch` on the instance, on the
component root, and a `min-width` floor under the whole page. The first two did
nothing and were removed rather than left in as decoration; the floor worked and
was a crutch — it made the page a fixed size instead of giving its contents a
size, which is the same mistake one level up. The owner said so, and was right.

**And underneath it, `AppShell` was placed in the window with no size at all**,
so it took its *preferred* size — the size of whichever page was showing. Every
page shorter than the window left a strip of nothing beneath it. It had been
there since M6, invisible for the same reason as the first fault: until this
milestone, every page happened to be big. The shell is now given `root.width`
and `root.height`.

One thing worth keeping from the wrong turns: three renders in a row came back
identical after two structural rewrites. Two changes that change nothing are
evidence about the model, not the markup — and the fourth render, forced after
touching `build.rs`, ruled out a stale build and left the assumption as the only
suspect standing.

## 53. The design document is not coming, because the design already happened

`PROJECT_MASTER.json` names `docs/design/DESIGN_PRINCIPLES.md` as the *input* to
M15: the owner would write the visual language down as markdown, and M15 would
translate it into tokens. That file has been a seven-line placeholder since M0.

The owner has now settled it: the design was decided screen by screen as the
windows were built, and what is on screen is what they want. So the document is
not an input that is late — it is a record that was written in the wrong medium.
Every decision it would have contained was made, argued and applied already:
four type sizes and nothing between them, three faces with three jobs, one
spacing series, even lengths everywhere, a line box tall enough for the face in
it, one button shape, hover that belongs to the whole object, a line where
something ends and a fade where it continues, and a single colour in either
palette.

**Chosen:** `docs/design/DESIGN_PRINCIPLES.md` becomes the written form of the
design that shipped rather than a brief for one that has not. It is descriptive,
and `crates/ui/slint/theme/tokens.slint` stays normative — a value lives in one
place, and that place is the one the compiler reads. The document explains why
each value is what it is, which is the part a file of numbers cannot carry.

**What this does to M15.** Five of its seven tasks are built and have been for
several milestones: design tokens, the component library, the EqStar dial, the
visualiser's style, and both themes. Two are not, and neither is cosmetic —
both are requirements of section 2.10 in their own right:

- **interface scaling**, which today follows the system DPI and has no setting,
- **drag-and-drop of tracks inside the application**, which does not exist at
  all: the only dragging in the interface moves the window, a dial arm and the
  playhead.

So M15 is not "done because the design is done". It is reduced to those two,
and they are what its definition of done — "дизайн интегрирован, ничего не
сломано" — now means. Recorded here rather than assumed, because a milestone
quietly declared complete over two unbuilt requirements is exactly the kind of
gap `MASTER_ISSUES` 51 was written about.

Section `16_Текущий_статус` of the master still reads "M0-M5 завершены" with
`next_step: M6`. That is the master's own field to correct and it is stale by
nine milestones; it is left alone here and flagged to the owner.

## 54. The daily rollups exist, and nothing writes them

Migration 7 creates `daily_track_stats`, `daily_artist_stats`,
`daily_genre_stats` and `daily_radio_stats` — section 7.4's aggregates — and M14
lists "aggregates" among its tasks. M14 did not write them. Both numbers the
dashboard shows are queried straight out of `play_events`: one row of totals and
the ten most-played files.

That was the right build and the wrong silence. A rollup is a cache of a query,
and this query runs over at most thirty days of one listener's events — a few
thousand rows with an index on `(profile_id, started_at)`. Maintaining four
tables to avoid it would cost a write on every listen, a second source of truth
about the same facts, and a repair path for when the two disagree. The
requirement M14 actually has to meet is that statistics are collected, expire and
can be switched off, and all three are met.

**Chosen:** the tables stay created and unwritten, and it is written down here
rather than left for somebody to discover a schema that lies about what is in
it. They are not dropped: migrations are append-only and a table that costs
nothing empty is not worth a migration to remove. They get written the day
something needs an answer that cannot be counted from thirty days of events —
per-station listening totals are the likeliest candidate, and that one is
blocked by something else first: `play_events.radio_session_id` is never filled,
because the queue knows which station is playing and `PlaybackService`, which
writes the event, does not.

Until then, "aggregates" in M14's task list means the aggregation, not the
tables.

## 55. What is wired, and what is only a seam

The owner asked whether anything in the code is not actually running, or is
swallowing errors, after nine milestones during which the window kept looking
correct. Checked rather than assumed: every port method against its call sites,
every discarded `Result`, every panic site, and every callback the window
declares.

**The window is correct for everything the listener does, and blind to
everything that happens behind them.** That is the one story under all of it,
and it is why nothing looks broken: today, almost nothing happens behind them.

**The file watcher does not run in the window.** `NotifyFileWatcher` is built,
tested and wired only into `cadenza watch`, whose own comment says "until the
interface exists in M6 this is the only way to see the watcher work". M6 came
and it was never wired in. The master asks for "автоматическое отслеживание
изменений файловой системы" and today that holds only for as long as a terminal
is open next to the window. A file dropped into a watched folder while Cadenza
is running is not noticed until somebody scans by hand.

**The event bus publishes to nobody.** Eight kinds of `DomainEvent`, sixteen
`publish` sites, and `subscribe` is never called outside its own unit test. The
interface refreshes because the controller re-reads a page when it is opened and
after each command it issues — which covers every change the listener causes and
no change caused by anything else. So the two findings are really one: wiring
the watcher without a subscriber would update the database under a window that
goes on showing the old library.

**Artwork is extracted, written to the cache and never read.** `store` is called
for every scanned file; `path_for` has no caller. The player bar draws a letter
where a cover would go. Nothing is broken by this — it costs scan time and a
cache that only grows.

**Five other seams have no caller** and are harmless: `get_session` (a station
is live in memory and does not survive a restart), `recent` (the dashboard
aggregates instead of listing), `delete_item` (playlist entries are renumbered
and rewritten in one transaction instead), `profile_remove`, and `supports` (the
decoder is asked to decode rather than asked first).

**Errors are not generally swallowed.** Failures from the services reach the
player bar through `Controller::report`; six `unwrap`/`expect` sites remain in
production code and each names a local invariant; the discarded `Result`s are
shutdown paths — joining threads, replying on a channel whose receiver has gone
— plus three deliberate ones: a queue that fails to save, artwork that fails to
cache, and a listen that fails to record. The first two are commented and are
right: neither is a reason to stop the music. The third is not commented and
should be.

**There is no logging of any kind** — no `log`, no `tracing`, nothing written to
the log path the app already computes. Whatever a background thread swallows is
therefore invisible, which is tolerable while the only background thread is the
analyser and intolerable the moment the watcher joins it. M16 lists "logs" among
its tasks; that is the milestone this belongs to.

**Recommended, and the owner's to schedule:** the watcher and a subscription are
one job and should be done together, because either alone is worse than neither.
Logging goes with M16 as planned. Covers are a feature rather than a fix. The
five idle seams stay: a port method with no caller costs a line of a trait, and
removing one would be a change to the architecture for tidiness.
