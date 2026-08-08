# 2. Slint for the UI

Status: accepted (M0)

## Context

The UI must be native desktop — explicitly no Electron, Tauri, WebView or HTML/CSS/JS.
It has to support a custom visual design delivered later as a markdown spec, dark and
light themes, and interface scaling.

## Decision

Slint, with all UI code confined to `crates/ui`.

## Consequences

- Declarative markup that maps well onto a design-token-driven theme, which is what M15
  needs.
- Renders natively; no browser engine is shipped.
- **Licensing:** Slint is used under GPLv3, so Cadenza is GPL-3.0-or-later. This is
  acceptable because the project is open source.
- If the project license ever changes, Slint must be swapped out. That stays feasible
  only because the UI layer holds no business logic and talks to the application layer
  through commands and view state alone — see `docs/UI_CONTRACT.md`. Do not erode that
  boundary for convenience.
