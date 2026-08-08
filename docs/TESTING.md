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
