# Cadenza

Local desktop music player for Windows. Fully offline: no internet, no telemetry, no
cloud accounts, no neural networks.

- Local library with folder scanning and file watching
- Playlists, manual queue, smart shuffle, local smart radio
- Crossfade and gapless playback
- 3-band and 10-band equalizer
- Lightweight audio visualization
- Local listening analytics with a 30-day retention window
- Multiple user profiles, each with its own library, playlists, history and settings

## Status

Milestone **M0 — scaffold** complete. See [docs/PROJECT_PLAN.md](docs/PROJECT_PLAN.md).

## Build

```bash
cargo build --workspace
cargo test --workspace
cargo run -p cadenza-app
```

Requires Rust stable (see `rust-toolchain.toml`).

## Layout

| Crate | Layer | May depend on |
|---|---|---|
| `crates/core` | domain + application | nothing in this workspace |
| `crates/infra` | SQLite, audio, metadata, filesystem, analysis | `core` |
| `crates/ui` | Slint views, view models, commands | `core` |
| `crates/app` | composition root | `core`, `infra`, `ui` |
| `crates/testkit` | shared test helpers | `core` (`infra` from M2) |

## Source of truth

`PROJECT_MASTER.json` at the repository root defines the requirements, stack,
architecture, data model and milestone plan. It outranks this README and every file
under `docs/`. Do not change the stack, layering, profile rules, history retention or
the offline/no-ML stance without amending it first.

## License

GPL-3.0-or-later (the Slint UI dependency is used under GPLv3; see
[docs/adr/0002-slint-ui.md](docs/adr/0002-slint-ui.md)).
