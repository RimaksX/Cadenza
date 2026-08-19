# UI contract

Normative rules: `PROJECT_MASTER.json`, sections `2.10_UI_и_будущий_дизайн` and
`4.3_Правила_UI`.

Status: the shell landed in M6 — window, sidebar, library listing, player bar,
two themes — and has since gained nine screens: library, queue, playlists, one
playlist, equaliser, radio, listening, decisions and settings. The visual
language they share is written down in
[design/DESIGN_PRINCIPLES.md](design/DESIGN_PRINCIPLES.md).

## The contract

The UI is a pure function of view state plus a command sink.

- **Down:** the application layer publishes immutable view state
  (`core/src/application/view_state.rs`, and `domain::track::TrackSummary` for
  listings). View models translate it into Slint models.
- **Up:** the UI calls application services. It never mutates domain state
  itself.
- **Sideways:** the UI subscribes to the event bus, and a published event tells
  it only *that* something changed. The handler runs on whichever thread
  published, so it raises a flag; the 250 ms tick reads the flag and re-reads the
  pages a change behind the listener's back could have altered. Nothing arrives
  through that channel except the prompt to read again.

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

Of those, both themes are delivered — stored on the profile, so a listener's
choice comes back on the next run — and so is the fixed layout.

Two are still missing, and they are the whole of what M15 has left
([MASTER_ISSUES 53](MASTER_ISSUES.md)):

- **Scaling** follows the system DPI and has no setting of its own, although
  there has been a settings screen to put one on since before M14.
- **Drag-and-drop of tracks** does not exist. The only dragging in the window
  moves the window itself, an equaliser dial and the playhead. Tracks reach a
  playlist or the queue through the row menu instead, which works and is not
  what 2.10 asks for.

## Where a component may look

Nothing in `slint/` names a colour, a size, a font or a spacing of its own: every
value comes from `theme/tokens.slint`, which picks between `theme/dark.slint` and
`theme/light.slint`. That is what keeps any change of look an edit to
three files rather than a sweep through every view (PROJECT_MASTER 14, and
[DESIGN_INTEGRATION.md](DESIGN_INTEGRATION.md)).

The palette is set from Rust — the controller reads the profile's theme and
writes `Theme.dark`. Markup may read that global and never assign to it.

## Contrast

Both palettes are measured rather than judged. Every ink that carries text
clears 4.5:1 against the surfaces it appears on, which for the smallest type in
the interface — the mono captions in `ink-mute` — is the difference between a
caption and a smudge:

| | dark | light |
|---|---|---|
| `ink-mute` on `bg-0` / `bg-1` | 4.82 / 4.64 | 4.92 / 4.63 |
| `ink-disabled` | 2.82 | 2.82 |
| `line` / `line-soft` | 1.91 / 1.40 | 1.92 / 1.40 |
| `danger` on `bg-2` | 5.27 | 5.46 |

`danger` is the only colour in either palette. It marks a control that takes
something away — remove from the library or from a playlist, delete a playlist
or a preset, replace the copy you already have, tell a station you want less of
this — and only while the pointer is on it, so the interface stays grey until
something is about to be destroyed. Warm rather than a signal red, so it belongs to this hue
instead of arriving from a system dialog; the light side is deeper than a
mirror of the dark one would give, because a light red on a near-white surface
reads as pink rather than as a warning.

`ink-disabled` is deliberately below the minimum. A disabled control is exempt
from it, and cannot look unavailable while it is as legible as the controls that
work — which is what happened the first time `ink-mute` was raised and the
disabled items came up with it.

Separators are not text and have no minimum, but below about 1.3 a hairline
stops being quiet and becomes absent. That is what the light theme's first
values did to the track list.
