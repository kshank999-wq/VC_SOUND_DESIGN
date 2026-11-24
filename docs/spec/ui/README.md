# VC Sound Design — UI handoff package (v2.1)

Nov 24, 2026 (v2.1: measured color and contrast fixes; see SPEC.md › Changes in v2.1). Everything a coding agent needs to build the VC Sound Design desktop UI.

## What's here

| Path | What it is | Use it for |
|---|---|---|
| `SPEC.md` | Full UI spec v2: layout, screens, the six-stage workflow, every button → engine function, components, state, shortcuts, acceptance checklist, function list | **Source of truth** for structure, behavior and which function each control calls |
| `tokens.css` | Design tokens v2 (also embedded in SPEC.md Appendix A) | **Source of truth** for color, type, radius, shadow, motion, sizes. No hard-coded values outside it |
| `mockups/edit-window.html` | Interactive mockup of the Edit window at 1680 × 1050. Open it in a browser | Visual target. Keys **1–6** or the stage bar switch stages |
| `mockups/screenshots/1-prep.png … 6-deliver.png` | One screenshot per stage | Quick visual reference and diffing |
| `mockups/source/EditWindow.dc.html` | Design-canvas source of the mockup | Exact values for any element (the inline styles are the spec in practice) |

Mockup URL options: `edit-window.html?stage=mix&accent=%23C026D3&readout=%23FFC266`.
The accent options are `#5B8CFF` (default), `#6366F1` and `#C026D3`. The readout
options are `#7CF5D8` (default), `#9BF07A`, `#FFC266` and `#C7D2FE`.

## Read in this order

1. `SPEC.md` → **Changes in v2** and **Changes in v2.1** (top of file). They override the older
   numbers further down.
2. `SPEC.md` → Layout, Workflow, Regions in detail, Components.
3. `tokens.css`.
4. Open `mockups/edit-window.html` and step through the stages.

## How the mockup maps to the build

- **The mockup is not production code.** It is a static HTML/CSS picture driven by a tiny template
  runtime. Rebuild it in the app's real stack (`ui/src/`, one store in `store.ts`, engine calls via
  `call(cmd, args)` in `api.ts`).
- **Draw the timeline on canvas** (ruler, marker lanes, clips, automation, playhead). The mockup
  uses absolutely positioned divs only to show the look. Everything else (panels, keys, inspector,
  deck) is ordinary DOM.
- **Stage switching only changes panel defaults.** That covers picture height, marker-lane mode,
  open folders, deck tab and height, rack tab and inspector section. See the table in Changes in
  v2. No project data changes.
- **The placeholder data is "Gravel Road", reel 1** at 24 fps from 01:00:00:00, with 2 scenes,
  13 markers and 11 cues. Two markers are uncued (HF-011, DN-013), one clip is offline (BG-006) and
  two are placeholders (HF-004, PR-010). Render real project data in the build.
- **The selection state differs per stage** only to show the inspector variants: no selection
  (Prep/Deliver), marker FS-009 (Spot), cue HF-007 (Build), cue FS-009 while performing (Perform)
  and track Feet (Mix).

## Visual rules that are easy to get wrong

- **Lit key:** an accent inner ring, a soft glow and a gradient built from `--accent`. Never a
  hard-coded rgba.
- **Mute and solo:** M filled zinc `--mute` with a dark legend; S filled `--solo`. Neither uses the
  accent.
- **Marker states:**
  - cued markers are solid;
  - uncued markers are 75 % opacity with a dashed outline (dashed stems for points);
  - repeated markers are a bar with dark tick lines at each tap.
- **Clip states:**
  - placeholder clips have a dashed `--placeholder` outline;
  - offline clips are dashed at 50 % opacity;
  - the selected clip has an accent ring, glow and fade handles at the top corners.
- **AI proposals:** a dashed `--ai` ring with a faint fill.
- **Playhead:** `--playhead` (pink-lilac) with its glow, not the readout color.
- **Status text:** `--error` and `--ok` are for icons and meter segments; labels use
  `--error-text` and `--ok-text`.
- **Glass:** use backdrop blur on panels only, never on the timeline canvas or the meters. The
  chassis gradient and grid must stay, or the glass reads as flat grey.
- **Button reset:** use `:where(.app) button { font: inherit }`, zero specificity, or it overrides
  `.key`.
- **Reduce motion:** set transitions to 0 ms and keep the glows. **Reduce transparency:** swap in
  `--surface-1-opaque` and set the blur to 0.

## Not designed yet (build from SPEC.md, same components)

Welcome, New project, Import summary sheet, Devices & Preferences, Spot bindings, Plugin manager,
Reports sheet, Updates sheet, and the narrow-window behavior below 1280 px (inspector as an overlay
sheet, rack as an icon rail).
