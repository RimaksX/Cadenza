# Design integration

Normative process: `PROJECT_MASTER.json`, section `14_Дизайн_интеграция`. Executed in
M15.

The final visual design arrives as a markdown document at
`docs/design/DESIGN_PRINCIPLES.md`. When it does:

1. Translate its principles into design tokens (`crates/ui/slint/theme/tokens.slint`).
2. Update the dark and light themes.
3. Update components and views.
4. Leave `core` and the application layer alone unless the design genuinely requires a
   new command or a new field in view state.
5. Keep existing commands and view states intact.

If step 4 or 5 cannot be honoured, that is an architecture change: record it in
`PROJECT_MASTER.json` before writing code.
