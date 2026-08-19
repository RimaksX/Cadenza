# Architecture

Normative definition: `PROJECT_MASTER.json`, sections `4_Архитектурный_стиль` and
`5_Полная_структура_проекта`. This file explains how the rules are enforced.

## Layers

Clean / hexagonal. Business logic knows nothing about the UI, the database, audio
libraries or the operating system.

- **Domain** (`core/src/domain`) — entities, value objects, policies, ports.
- **Application** (`core/src/application`) — use cases, commands, events, services,
  view state.
- **Infrastructure** (`infra`) — SQLite, audio engine, decoder, metadata reader, file
  watcher, analyzer, system utilities. Implements the ports declared in the domain.
- **UI** (`ui`) — Slint views, view models, theme, design tokens.

## Dependency rules

Allowed: `app -> core`, `app -> infra`, `app -> ui`, `ui -> core`, `infra -> core`.

Forbidden: `core -> infra`, `core -> ui`, `ui -> infra`.

Enforcement is structural: the forbidden edges are simply absent from the
`[dependencies]` sections of `crates/core/Cargo.toml` and `crates/ui/Cargo.toml`, so
violating one fails to compile. Keep it that way.

## What the UI may and may not do

May: render view state, invoke commands, subscribe to events.

May not: touch the database, read files directly, drive the audio stream, hold business
rules, or contain radio/shuffle/history algorithms.

## Ports are synchronous

No async runtime is part of the stack. rusqlite, cpal and Symphonia are blocking
APIs, and a desktop player has no concurrency load that would justify one. Long-running
work — scanning, hashing, feature extraction — runs on ordinary threads. Every port is
`Send + Sync` so those threads can share it.

Those threads stay out of the way by resting rather than by being demoted.
`SystemPriorityPort` exists and its Windows implementation deliberately does
nothing: `SetThreadPriority` needs `unsafe` or a dependency taken for one
function, and neither buys the guarantee that 2.11 actually asks for. What keeps
background work under about a fifth of the machine is `analysis_policy`'s duty
cycle — the worker rests four times as long as it works. A share of the clock is
a promise that does not depend on a scheduler agreeing with it; the port stays as
the seam for the day the call is worth making.

## Composition

`crates/app` is the only place allowed to construct concrete infrastructure and hand it
to the application layer as port implementations.

`AppContext` carries only the ports that exist. It gains a field per milestone rather
than declaring all twenty now and forcing each milestone to stub the ones it does not
have yet.

## Where the master file was wrong

Contradictions and gaps found in `PROJECT_MASTER.json` during implementation, and the
option chosen for each, are recorded in [MASTER_ISSUES.md](MASTER_ISSUES.md).
