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
