# Project plan

Milestones and their definition of done are specified in `PROJECT_MASTER.json`
section `11_План_реализации`. This file tracks progress only.

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold проекта | done |
| M1 | Core skeleton | done |
| M2 | SQLite infrastructure | not started |
| M3 | Профили и настройки | not started |
| M4 | Библиотека и сканирование | not started |
| M5 | Базовый audio engine | not started |
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
