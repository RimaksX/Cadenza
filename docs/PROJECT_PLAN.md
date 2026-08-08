# Project plan

Milestones and their definition of done are specified in `PROJECT_MASTER.json`
section `11_План_реализации`. This file tracks progress only.

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold проекта | done |
| M1 | Core skeleton | not started |
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
- `LICENSE` — needs the verbatim GPLv3 text
- `packaging/`, `resources/` — M15/M16
