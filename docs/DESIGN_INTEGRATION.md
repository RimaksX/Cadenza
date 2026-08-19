# Design integration

Normative process: `PROJECT_MASTER.json`, section `14_Дизайн_интеграция`.

That section describes a design that arrives as a markdown document and is then
translated into tokens. It did not happen that way: the design was settled screen
by screen as the windows were built, and the owner has confirmed that what is on
screen is what they want ([MASTER_ISSUES 53](MASTER_ISSUES.md)). The document is
therefore a record rather than a brief — [design/DESIGN_PRINCIPLES.md](design/DESIGN_PRINCIPLES.md).

What section 14 was actually protecting is still in force, and it is this: **the
design lives in three files, so changing it is an edit rather than a sweep.**

1. Values live in `crates/ui/slint/theme/tokens.slint`, which picks between
   `theme/dark.slint` and `theme/light.slint`. Nothing in `slint/` names a
   colour, a size, a font family or a spacing of its own.
2. Components and views read tokens and nothing else.
3. `core` and the application layer are untouched by a visual change. If a change
   genuinely needs a new command or a new field of view state, that is an
   architecture change: record it in `PROJECT_MASTER.json` before writing code.

The palette is set from Rust — the controller reads the profile's `theme` column
and writes `Theme.dark`. Markup may read that global and never assign to it.

Any later change to the look — the owner's, or a themeing feature — goes through
the same three steps, and `python scripts/audit_ui.py` is run afterwards.
