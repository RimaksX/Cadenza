# Cadenza

Local desktop music player for Windows. Fully offline: no internet, no telemetry, no
cloud accounts, no neural networks.

- Local library with folder scanning and file watching
- Playlists, manual queue, smart shuffle, local smart radio
- Crossfade and gapless playback
- Three-knob and eight-band parametric equalizer
- Lightweight audio visualization
- Local listening analytics with a 30-day retention window
- Multiple user profiles, each with its own library, playlists, history and settings

## Status

Milestones **M0 – M15** complete: everything in the feature list above works, in
a window with nine screens, two themes and four sizes. **M16**, Windows
packaging, has not started.

What was actually built in each milestone is in
[docs/PROJECT_PLAN.md](docs/PROJECT_PLAN.md); every deviation from the master file
and the reason for it is in [docs/MASTER_ISSUES.md](docs/MASTER_ISSUES.md).

## Build

```bash
cargo build --workspace
cargo test --workspace
cargo run -p cadenza-app
```

Requires Rust stable (see `rust-toolchain.toml`).

`target/` grows: cargo keeps every artefact it has ever built, and on Windows
each carries a `.pdb` beside it. The dev profile keeps line tables for this
project and no debug information at all for dependencies, which is what holds a
full build with tests to about four gigabytes rather than forty
([docs/MASTER_ISSUES.md](docs/MASTER_ISSUES.md), finding 71). `cargo clean`
returns all of it.

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

GPL-3.0-only — version 3 of the GNU GPL and no other, because that is the
version the Slint UI dependency is used under (see
[docs/adr/0002-slint-ui.md](docs/adr/0002-slint-ui.md)).
