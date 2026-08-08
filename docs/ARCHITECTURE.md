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

## Composition

`crates/app` is the only place allowed to construct concrete infrastructure and hand it
to the application layer as port implementations.
