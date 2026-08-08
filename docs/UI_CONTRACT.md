# UI contract

Normative rules: `PROJECT_MASTER.json`, sections `2.10_UI_и_будущий_дизайн` and
`4.3_Правила_UI`.

Status: not implemented. The shell lands in M6.

## The contract

The UI is a pure function of view state plus a command sink.

- **Down:** the application layer publishes immutable view state
  (`core/src/application/state/*`) and events. View models translate it into Slint
  models.
- **Up:** the UI emits commands (`core/src/application/commands.rs`). It never mutates
  domain state itself.

Consequences:

- No SQL, no file IO, no audio calls, no business rules in `crates/ui`.
- No shuffle, radio, history or statistics logic in the UI, not even "just for display".
- The UI is not the owner of application state; it can be closed and rebuilt without
  losing anything.

This is what makes the UI layer replaceable, which section `2.12_Лицензия` requires in
case the project license changes.

## Presentation requirements

Native desktop only — no web, WebView, Electron or Tauri. Dark and light themes,
interface scaling, in-app drag-and-drop of tracks, and a fixed (not user-rearrangeable)
layout.
