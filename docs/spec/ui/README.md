# VC Sound Design — UI package for the design agent

This package defines the layout, screens, workflow, buttons and functions of the VC Sound Design
desktop app, for a UI design agent to design from and a coding agent to build from.

The app uses the same five-band layout as the VC Composer 2 DAW (top deck, toolbar,
rack / edit window / inspector, bottom deck, status strip), with a more modern tech look: glass
panels over a softly lit backdrop, hairline borders, crisp glowing readouts and one themeable
accent color (indigo by default). v2 fixes the v1 review: tokens that follow the theme, measured
color separation and contrast, per-stage layouts with a height budget, a taller toolbar and one
mixer.

## Start here

1. Read `SPEC.md` end to end: layout, screens, the six-stage workflow with every button and the
   engine function it calls, components, shortcuts and the acceptance checklist.
2. Import `tokens/tokens.css` as the theme before designing or building any component.
3. For the shared layout and component vocabulary, see VC Composer 2's UI package:
   `kshank999-wq/vc_composer2` → `docs/spec/ui/` (`SPEC.md`, `mockups/`, `tokens/`).

## What's in the box

| Path | What it is |
|---|---|
| `SPEC.md` | The full UI spec: source of truth for structure, functions, sizes and behavior |
| `tokens/tokens.css` | CSS custom properties and component recipes: source of truth for colors, type and depth |

## Ground rules

- Every function in the spec exists today as an engine command (`crates/vcsd-engine/src/lib.rs`,
  called through `ui/src/api.ts`). Design controls for them; don't add back-end behavior.
- The current UI in `ui/src/` is the working reference for each command's arguments and for
  behavior details (snapping, automation writing, import summaries).
- Mockups are not in this package yet. Ask for them, or produce them as the first deliverable:
  the Edit window in each of the six stages, at 1680 × 1050.
