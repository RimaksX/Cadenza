# UI contract

Normative rules: `PROJECT_MASTER.json`, sections `2.10_UI_и_будущий_дизайн` and
`4.3_Правила_UI`.

Status: the shell landed in M6 — window, sidebar, library listing, player bar,
two themes.

## The contract

The UI is a pure function of view state plus a command sink.

- **Down:** the application layer publishes immutable view state
  (`core/src/application/view_state.rs`, and `domain::track::TrackSummary` for
  listings). View models translate it into Slint models.
- **Up:** the UI calls application services. It never mutates domain state
  itself.

The up-channel is the services rather than a command enum, and the reasoning is
in [MASTER_ISSUES.md](MASTER_ISSUES.md) finding 30. `PlaybackService::toggle` is
a command; giving it a second name in an enum that a dispatcher immediately
matches back into the same call would add ceremony and remove nothing. An enum
earns its place when a command has to be queued, retried or undone.

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

Of those, M6 delivers both themes — stored on the profile, so a listener's choice
comes back on the next run — and the fixed layout. Scaling follows the system DPI
and gains a setting when there is a settings screen to put it on; drag-and-drop
needs playlists to drag into, which is M7.

## Where a component may look

Nothing in `slint/` names a colour, a size, a font or a spacing of its own: every
value comes from `theme/tokens.slint`, which picks between `theme/dark.slint` and
`theme/light.slint`. That is what makes the design document of M15 an edit to
three files rather than a sweep through every view (PROJECT_MASTER 14).

The palette is set from Rust — the controller reads the profile's theme and
writes `Theme.dark`. Markup may read that global and never assign to it.
