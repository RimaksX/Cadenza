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

## 10. Genre edits leak between profiles — **open**

Section 2.1 lets a profile edit metadata locally, and 12.1 forbids profile data
leaking. But `track_genres` in section 7.2 is keyed on `media_file_id` alone, so
one profile correcting a genre changes it for every profile sharing that file.

Three options, none free:

1. Add a per-profile genre override table. Correct, costs a table and a join on
   every genre query.
2. Accept it: genre is a property of the recording, not of the listener. Cheapest,
   but contradicts 2.1 as written.
3. Keep the tag-derived genre global and add profile-scoped *tags* separately.
   Most flexible, most work.

Needs a decision before M4 implements metadata editing. Option 1 is the one that
honours both rules as written.

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

## 13. Symphonia's AAC and ALAC coverage — risk, not a contradiction

Section 2.2 requires AAC and ALAC; section 3.4 fixes Symphonia as the decoder.
Symphonia's AAC support is limited to AAC-LC, and gapless MP3 needs LAME/Xing
delay and padding tags to be read correctly.

Not a decision for M1, but M5 must verify both against real files before the
audio engine is called done. If coverage falls short, `DecoderPort` is the seam an
alternative adapter plugs into (ADR 0005) — the stack does not need to change.

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

## 17. Section 16 status is stale — **open**

`16_Текущий_статус` still reads "Реализация кода еще не начата" and
`next_step: M0`. M0 and M1 are complete.

Keeping it current is maintenance rather than an architecture change, but the
master file is the source of truth and should not be edited as a side effect of
implementation work. Awaiting confirmation before updating it.
