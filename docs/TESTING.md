# Testing

Normative requirements: `PROJECT_MASTER.json`, section `13_Тестирование`.

## Required coverage

- Unit tests for domain policies and value objects.
- Integration tests for SQLite repositories, against a temporary database.
- Integration tests for the library scanner.
- Tests for queue, shuffle and radio selection logic.
- Tests for the history policy (>50% counts as a play, <10 s counts as a skip, the band
  in between counts as neither).
- Smoke tests for UI state mapping.
- Tests for the basic DSP components.

## Determinism

Anything time-dependent goes through `ClockPort` and uses the test clock from
`crates/testkit`, never `SystemTime::now()` inside domain code. Anything random —
shuffle, radio exploration noise — takes a seed so tests can pin it.

## Fixtures

Audio and metadata fixtures live in `tests/fixtures/audio/` and
`tests/fixtures/metadata/`: WAV, FLAC, MP3, and M4A/AAC where feasible. Keep them small;
they are committed to the repository.

## Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs exactly these on `windows-latest`.

## The temporary database fixture

`TempDb` names its directory after the process id, a start-time stamp and a
counter. The stamp is not decoration: Windows reissues process ids, and a
removal that failed on an earlier run leaves a database behind for the next
process given that number — which then opens somebody else's data and fails on
a unique constraint, a very long way from anything the test is about. That is
what it did, in a suite that had accumulated some eighteen hundred leftover
directories.

Every fixture in a test harness struct is declared **last**. Fields drop in
declaration order, and the database cannot delete its directory while a
repository above it still holds a connection into it. Getting that order wrong
is why the directories were left behind in the first place.

## The interface audit

`python scripts/audit_ui.py` asks five questions of every line of Slint markup
and exits non-zero if any of them has an answer. Each check is there because
the defect it names shipped once and had to be found by eye: type with no line
box of its own, a box too small for the type in it (Slint drops such a line
rather than clipping it), a boxed line standing directly in a horizontal layout
where it cannot stretch and so rides at the top of its row, a halving that was
never rounded, and any odd length at all.

It is not part of `cargo test` — it reads markup, not Rust — so run it after
any change to `crates/ui/slint/`.

What it cannot see is everything that needs a live pointer: hover, drag, focus
and the click itself. Those are checked by running the binary and looking, and
what cannot be checked that way is said plainly rather than implied.
