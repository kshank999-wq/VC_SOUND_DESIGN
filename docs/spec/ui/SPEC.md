# VC Sound Design — UI Spec for the design agent

v2 · Nov 24, 2026 (v1 review fixes: theme-following tokens, color collisions, text contrast,
per-stage layouts with a height budget, a lit backdrop for the glass, a taller toolbar, one mixer)

## Overview

Redesign the VC Sound Design desktop app: a picture-locked sound-design and Foley DAW where
editors spot events against picture, turn them into cues, build tracks from a sound library,
perform and record, mix, and deliver to editorial (AAF, OTIO, stems, cue sheets).

**Layout:** use the same structure as the VC Composer 2 DAW. That means:
- a top deck of transport and readouts;
- a toolbar of grouped keys;
- three columns: left rack, edit window and inspector;
- a tabbed bottom deck;
- a thin status strip.

Its reference package is `kshank999-wq/vc_composer2` → `docs/spec/ui/` (`SPEC.md`, mockups,
tokens). Reuse its component vocabulary: panels, keycaps with LEDs, backlit readouts, knobs, meters
and a stage bar.

**Look:** a more modern tech version of that hardware console:
- glass panels over a near-black chassis, with hairline borders instead of screws and bevels;
- crisp monospace readouts with a soft glow;
- one accent color, and motion that feels instant.

The rest of this spec is the source of truth for structure, functions, sizes and behavior.
`tokens/tokens.css` is the source of truth for colors, type and depth.

- **Desktop first:** reference window 1680 × 1050, minimum 1280 × 800. Like VC Composer, it is a
  dense pro tool, so controls are smaller than 44 px touch targets on purpose. The mixer can be
  detached into its own window for a second monitor.
- **Same app, new skin and structure:** every function listed here already exists as an engine
  command (`crates/vcsd-engine/src/lib.rs`), called from the UI through `call(cmd, args)` in
  `ui/src/api.ts`. The current UI (`ui/src/`) shows the arguments each one takes. Design for these
  functions; don't invent new back-end behavior.
- **Placeholder content for mockups:** a short film, "Gravel Road", reel 1 at 24 fps from
  01:00:00:00, with two scenes and about twelve cues across the default lanes. Render real project
  data in the build.

## What carries over from VC Composer, and what's more modern

| | VC Composer 2 | VC Sound Design |
|---|---|---|
| Bands | Top deck · toolbar · rack / edit / inspector · keyboard deck · status | Same five bands |
| Workspace switch | Composer \| Producer on the nameplate | **Stage bar**: Prep › Spot › Build › Perform › Mix › Deliver (one data model; stages change tools, not data) |
| Edit lanes | Notation staves or MIDI/audio regions | Picture viewer above a **marker timeline** (spotting lanes) above **audio tracks** with automation lanes |
| Bottom deck | 88-key keyboard / mixer | **Pads / Library / Mixer** tabs |
| Chassis | Flat near-black | A lit backdrop: soft radial glows tinted from the accent and readout colors over a dark gradient, plus a faint 22 px dot grid (`--chassis-bg`, `--chassis-grid`). It gives the glass something to blur |
| Panels | Anodized gradient, screws, bevel | Glass (`--surface-1` at 72 % over a 14 px backdrop blur), 1 px hairline, a 1 px top sheen; no screws |
| Keys | Raised keycap, 2 px drop, LED glow when lit | Flat rounded key (8 px radius), 1 px hairline; lit = accent inner ring + soft glow; optional 6 px LED dot |
| Readouts | LCD with a retro mono face | Deep-black readout, JetBrains Mono with tabular figures, soft glow in `--readout` |
| Accent | Amber `--led` | Indigo `--accent` (themeable: indigo, violet, magenta, mono) |
| Motion | Mostly static | 120–200 ms ease-out on hover/press/panel changes; playhead and meters at display refresh |

## Design tokens

Use `tokens/tokens.css` before building any component. No hard-coded colors, radii, fonts or
shadows outside it.

- **Themeable at runtime** (Preferences), set as attributes on the root `<html>` element only:
  - `data-accent`: `indigo` (default), `violet`, `magenta`, `mono`;
  - `data-readout`: `mint` (default), `green`, `amber`, `ice`;
  - `data-density`: comfortable (default) or `compact`;
  - `data-transparency="reduced"` for opaque panels (the OS setting does the same).
- **Everything derived from the accent follows it:**
  - the lit-key fill and ring, glows, selection tint, the filled primary button and the backdrop
    glow are all built from `--accent` with `color-mix()`;
  - switching the theme changes them all, with no other values to update;
  - never hard-code an rgba of the accent.
- **Why amber and cyan aren't accent options:** they can't be kept apart from Solo, Music and
  Backgrounds (see below).
- **Categories** (`--cat-*`) color marker lanes, cue badges, track stripes and clip tints, with
  dark legends (`--on-category`). A lane, marker or track can override them with its own color.
  - **Engineering note:** new projects should get these as their default lane colors.
    `spotting::create_default_lanes` still uses the older, darker set; existing projects keep
    theirs.
- **Fonts:** Barlow Condensed for labels and key legends (uppercase, tracked), Inter for UI text,
  JetBrains Mono for every number that changes. All are free on Google Fonts.
- **Requires `color-mix()`:** Chromium 111+, WebKit 16.2+ (WebKitGTK 2.40+ for the Linux desktop
  build).

### Color rules (measured)

Colors that share a surface must be told apart. The rules below are measured as CIEDE2000
differences (ΔE: about 2 is barely visible, 10 clearly different, 20+ unmistakable). Contrast is
WCAG.

| Rule | Minimum | Measured |
|---|---|---|
| Categories against each other (lanes) | ΔE 15 | 21.5 |
| Accent (every theme) against every category (selection outlines on clips) | ΔE 15 | 17.9 (mono) to 33.1 |
| Accent against Mute and Solo (lit key vs. filled key) | ΔE 20 | 25.7 |
| Record against every category (recording region in a lane) | ΔE 15 | 21.1 |
| Mute / Solo / Record against each other | ΔE 20 | 40.5 |
| AI ring and playhead (white) against every category | ΔE 15 | 21.5 |
| Caution against Solo (meter amber next to a lit Solo key in a strip) | ΔE 20 | 20.9 |
| Meter green (`--ok`) against Footsteps | ΔE 10 | 15.7 |
| Text (`--text`, `--text-dim`) on every surface, hover included | 4.5:1 | 5.3:1 |
| Accent ring on a key (non-text control) | 3:1 | 3.3:1 (magenta) to 12.7:1 |
| Legend on the filled primary button | 4.5:1 | 6.1:1 |
| Dark legend on any category, Mute, Solo or Record fill | 4.5:1 | 5.3:1 (Record) |

**Kept apart by shape as well as color:**
- **AI proposal:** a dashed white ring plus an "AI" badge, never just a hue.
- **Recording region:** diagonal hatch.
- **Placeholder clip:** dashed outline.
- **QA states:** always an icon (✓ ⚠ ✕) next to their color.

**Never on the same surface:** `--caution` and the Dialogue yellow (ΔE 5.9). Caution appears only
in meters and in QA rows, as an icon; QA rows show categories as labelled badges.

**Not for text:** `--decor` (about 3:1) is for dividers, ticks and disabled outlines only, and
`--error` is for icons only.

## Layout

Five horizontal bands. Sizes are at the 1680 px reference width.

| # | Region | Size | Contents (left to right) |
|---|---|---|---|
| 1 | Top deck | 80 px | Nameplate (project name, dirty dot, ☰ project menu) · Transport · Timecode counter · Session readouts · Master meters |
| 2 | Toolbar | 52 px | Stage bar · Edit tools · Snap/Grid · Spot mode · Zoom · Undo/Redo · Export. Each group: a 12 px label, 4 px gap, 28 px keys, 4 px padding above and below |
| 3a | Left rack | 264 px, resizable 200–420 | Tabs: **Scenes** (script breakdown) · **Identities** · **Library** · **Markers** |
| 3b | Edit window | Fills the rest (min 640) | Picture viewer (upper, resizable split) over the **timeline**: ruler, marker lanes, track lanes, bus lanes. Center tabs: Timeline · Cue chart · QA & Export |
| 3c | Inspector | 300 px, resizable 240–460 | Context panel for the selection: cue, marker, clip, track, folder, bus, or project |
| 4 | Bottom deck | 132–300 px by tab and stage, or collapsed to its 28 px tab bar | Tabs: **Pads** (perform) · **Library browser** (waveform audition) · **Mixer** (strips; the only mixer, detachable) · **Log** |
| 5 | Status strip | 20 px | Sample rate · frame rate · CPU / RAM / disk streaming · audio device · MIDI in LED · autosave time · update pill |

Panel edges between 3a / 3b / 3c, the picture/timeline split and the bottom deck's top edge are
draggable; double-click restores the stage's default. `layout.get` / `layout.set` persist the
changes per stage and per project. ☰ → *Reset window
layout* restores everything.

Under 1280 px wide, the inspector becomes an overlay sheet (toggle key in the toolbar), and the
left rack collapses to an icon rail.

### Height budget (1050 px window)

The fixed chrome costs 152 px: top deck 80, toolbar 52 and status strip 20. Everything else
depends on the stage, so each stage has its own default layout:
- **Marker lanes** come in three heights:
  - full: 26 px per lane;
  - compact: 16 px per lane;
  - merged: every category on one 26 px lane, markers in their colors.
- **The picture** can be docked above the timeline, or shown as picture-in-picture: a 320 × 180
  overlay in the timeline's top-right corner, draggable, toggled with **P**.
- **The bottom deck** can be open at a size set per tab, or collapsed to its 28 px tab bar.

Track rows are 72 px (56 compact), plus automation lanes when shown.

| Stage | Bottom deck | Picture | Ruler | Marker lanes | Left for tracks | Tracks visible |
|---|---|---|---|---|---|---|
| Prep | Library, 220 | Docked, 320 | 66 | Merged, 26 | 1050 − 152 − 220 − 320 − 66 − 26 = **266** | 3 |
| Spot | Pads, 132 | Docked, 420 | 66 | Full, 7 × 26 = 182 | **98** | 1 (folders collapsed): Spot is picture and markers |
| Build | Collapsed, 28 | Docked, 280 | 66 | Compact, 7 × 16 = 112 | **412** | 5 (7 compact) |
| Perform | Pads, 220 | Docked, 400 | 66 | Merged, 26 | **186** | 2: the performed cue's track and its takes |
| Mix | Mixer, 300 | PiP | 66 | Merged, 26 | **506** | 7 (9 compact); collapse the deck for 10 |
| Deliver | Collapsed, 28 | Hidden | — | — | **870** for the QA & Export tab | — |

At 1050 px Spot is deliberately picture-first, and Mix is deliberately timeline-and-mixer-first.
The user's changes to a stage's layout are remembered for that stage. ☰ → *Reset window layout*
restores every stage's default.

### Edit window internals

- **Ruler** (66 px): three 22 px rows.
  - **Timecode:** HH:MM:SS:FF ticks at the frame rate, with the loop range as an accent bar along
    the bottom.
  - **Scenes:** scene blocks with number and title.
  - **Markers:** flags for picture-version change notes and score cues.
- **Marker lanes** (26 px each) sit above the tracks: one per spotting category (Footsteps, Props,
  Hard FX, Backgrounds, Design FX, Dialogue notes, Music, plus custom).
  - **Points:** a 3 px stem with a triangular cap.
  - **Ranges:** rounded bars.
  - **Repeated events:** a bar with tick marks at each tap.
  - **Color:** the marker's own color, else its lane's.
  - **State:** a cued marker is solid; an uncued one is 75 % opacity with a dashed outline.
- **Track rows:** each is a 232 px header plus the lane.
  - **Heights:** 72 px by default, 112 px tall, 56 px compact.
  - **Folders:** rows are grouped in collapsible folders. A collapsed folder shows all its clips on
    one row.
  - **Automation lanes:** each track can show its own (Volume, Pan, and any plugin parameter), 44 px
    each, under the track.
- **Bus section** below the tracks: stem buses and the master, with their own automation lanes.
- **Playhead:** a 1.5 px `--playhead` line through the ruler and every lane, with a cap in the
  timecode row.
- **Pinned and scrolling:** track headers stay pinned on horizontal scroll, and the ruler scrolls
  with the lanes.

## Screens

| Screen | Kind | Opens from | Purpose |
|---|---|---|---|
| **Welcome** | Full window | Launch with no project | New project, Open, Recover autosave, Open demo, recent projects |
| **Edit window** | Main | Always | Everything in the layout above; tools change with the stage |
| New project | Dialog | Welcome, ☰ | Name, folder, frame rate (23.976 … 60, drop-frame), sample rate, start TC |
| Project settings | Dialog | ☰ | Name, rates, start TC, contacts, AI settings |
| Import picture | Dialog | ☰, Prep stage | Video file, picture version label; proxy creation progress |
| Import AAF / Import OTIO | Dialog + result sheet | ☰, Prep stage | File, place at file TC or at a typed TC, bus levels (auto / restore / fold), media search folders; then the import summary (tracks, clips, markers, cues restored, scenes added, inserts, buses, missing media, notes) |
| Import VC Composer score | Dialog | ☰, Prep stage | Score JSON; TC / rate / scene mismatch warnings |
| Import VC Film Studio breakdown | Dialog | ☰, Prep stage | Breakdown JSON; conflicts to resolve |
| Relink missing media | Dialog | Inspector warnings, ☰ | Search folder; results per file |
| Add recorded take | Dialog | Cue inspector, Perform stage | WAV file for the selected cue |
| **Cue chart** | Center tab | Toolbar, center tabs | Sortable, filterable table of every cue (code, scene, category, identity, TC in/out, status, owner, priority, media state); click selects and locates |
| **Mixer** | Bottom deck tab; ⧉ detaches it into its own window | Toolbar *Mixer*, Ctrl/⌘ M, Mix stage | One strip per track and bus, master strip outlined in accent. There is one mixer: while it's detached, the deck tab says "Mixer is in its own window · Bring back". The Track inspector shows the selected track's controls, not a second mixer |
| **QA & Export** | Center tab | Toolbar *Export*, Deliver stage | QA run with findings; export profile and options; output file list; reports |
| Plugin manager | Dialog | ☰, insert slot "+" | Scan folders, list VST3/CLAP/AU, add to slot |
| Plugin editor | Floating window | Double-click an insert | The plugin's own editor or a generic parameter list |
| Devices & Preferences | Dialog | ☰, status strip device name | Audio device, buffer, MIDI inputs, test tone; theme (accent, readout, density); spot key bindings |
| Spot bindings | Sheet in Preferences | Spot mode key long-press | Map keys and MIDI notes to categories and marker kinds |
| AI proposals | Inspector section + review sheet | Cue inspector, Build stage | Run AI, then accept or reject each proposal |
| Reports | Sheet | Deliver stage | Cue sheet / summary as PDF, CSV, JSON; "active at" report |
| Updates | Sheet | Status strip pill | Release notes, install |

## Workflow (the stage bar)

The stage bar sits in the toolbar's left group: **Prep › Spot › Build › Perform › Mix › Deliver**.
- The current stage is lit in accent.
- Finished stages show a green LED. A stage counts as done when its checklist is met, for example
  "every marker is a cue" for Spot.
- Clicking a stage reconfigures the toolbar, the left rack's default tab, the inspector's default
  section, the bottom deck, the picture and the marker lanes, as in the height budget table.
- No data changes when switching stages, and every function stays reachable from any stage through
  ☰ or the context menus.

### 1. Prep: set up the project and bring material in

| Step | Buttons | Function |
|---|---|---|
| Create or open a project | New, Open, Recover, Demo | `project.new`, `project.open`, `project.recover`, `project.demo` |
| Set rates and start TC | Project settings | `project.settings` |
| Bring in picture | Import picture | `picture.import`, `picture.proxy` |
| Bring in the script breakdown | Import breakdown | `script.import` |
| Bring in the score | Import score | `score.import` |
| Bring in an editor's timeline | Import AAF, Import OTIO | `aaf.import`, `otio.import` |
| Add scenes | + Scene (Scenes tab), drag on the Scenes ruler row | `scene.add`, `scene.update` |
| Load sounds | Import folder, Install starter library | `library.import`, `library.install_starter` |
| Set up audio and MIDI | Devices | `devices.list`, `devices.status`, `prefs.set`, `devices.test_tone`, `midi.connect`, `midi.disconnect` |

Default panels: left rack **Scenes**, bottom deck **Library** (220 px), picture docked at 320 px.

### 2. Spot: mark every sound event against picture

| Step | Buttons / gestures | Function |
|---|---|---|
| Play and step picture | Play/Stop, ◀ ▶ frame step, locate by typing TC | `transport.toggle`, `transport.step`, `transport.locate`, `tc.parse` |
| Live spot while playing | **Spot** key (lit = on); F P G B D N M taps; Shift+key or Enter closes a range | `input.mode`, `spot.tap`, `spot.finish` |
| Draw markers | Drag in a lane = range; double-click = point; Repeated kind from the lane menu | `marker.add` |
| Adjust | Drag a marker or its edges; edit taps | `marker.update` |
| Color | Swatch in the marker inspector; *Use lane's* | `marker.color` |
| Remove | Delete key, inspector Delete | `marker.delete` |
| Remap keys and MIDI pads | Spot bindings | `spot.bindings`, `spot.bind` |
| Name sounds | + Identity (Identities tab) | `identity.add` |
| Turn markers into cues | **Make cues** (toolbar, marker inspector) | `cues.build` |

Default panels: left rack **Markers**, inspector **Marker**, bottom deck **Pads** at 132 px (spot
keys shown as pads in category colors), picture docked at 420 px, marker lanes full. While Spot mode is on, the toolbar shows a banner, "SPOT —
open: F, B (Enter closes)", in accent.

### 3. Build: put sounds on the cues

| Step | Buttons | Function |
|---|---|---|
| Fill every cue with placeholder sounds | **Populate** (whole project / scene / category per scene / identity everywhere) | `populate` |
| Search and audition | Library search field, ▶ on a result, Stop | `library.search`, `audition`, `audition.stop`, `asset.peaks` |
| Choose a sound for a cue | Drag a library result onto a cue; **Replace** (this cue / scene / all) | `cue.replace` |
| Go back | Revert to placeholder | `cue.revert` |
| Give an identity its sound | Assign (Identities tab) | `identity.assign` |
| Edit cue details | Description, In/Out, identity, status, priority, owner, notes | `cue.update` |
| Delete a cue | Inspector Delete | `cue.delete` |
| Edit clips | Drag to move; drag edges to trim; corner handles for fades; Split at playhead; Delete; gain, fill (one-shot / loop), mute | `clip.update`, `clip.split`, `clip.delete` |
| Ask AI | Run AI on the cue or scene; Accept / Reject each proposal | `ai.run`, `ai.accept`, `ai.reject` |

Default panels: left rack **Library** (with audition and waveform), inspector **Cue**, bottom deck
collapsed, so five tracks fit.

### 4. Perform: record Foley and performed effects

| Step | Buttons | Function |
|---|---|---|
| Choose the cue to perform | **Perform** in the cue inspector | `perform.select`, `perform.start` |
| Trigger sounds live | Pads J K L ; U I O P (Shift = accent), MIDI pads | `perform.trigger`, `midi.message` |
| Keep or drop the pass | **Commit** / **Cancel** | `perform.commit`, `perform.cancel` |
| Record a microphone | **Record** (red), Stop | `record.start`, `record.stop` |
| Add a take from a file | Add take | `take.add` |
| Loop a section | Loop key, Shift-drag on the ruler | `transport.loop` |

Default panels: bottom deck **Pads** (220 px), picture docked at 400 px, inspector **Cue** (takes
list).

### 5. Mix: balance, automate and process

| Step | Buttons | Function |
|---|---|---|
| Track level, pan, mute, solo, color, folder, bus, height | Header knobs / keys; inspector fields; header swatch (Alt-click resets) | `track.update` |
| Folders | Move, rename, ungroup; folder M / S | `folder.move`, `folder.rename`, `folder.ungroup`, `folder.flag` |
| Buses | Level, mute, balance (bus strips) | `bus.update` |
| Automation | **A** key shows lanes; R / T / L / W mode key; click to add a point; drag to move; double-click to delete; Clear lane | `automation.set`, `automation.update`, `automation.delete`, `automation.clear`, `automation.mode`, `automation.touch` |
| Inserts | Slot "+" (plugin list), bypass LED, drag to reorder, remove | `plugins.list`, `plugins.scan`, `plugins.folders`, `insert.add`, `insert.move`, `insert.bypass`, `insert.remove` |
| Plugin parameters | Plugin editor knobs, automate a parameter | `plugin.params`, `plugin.param` |

Default panels: center tab **Timeline**, bottom deck **Mixer** (300 px), picture as
picture-in-picture, marker lanes merged, inspector **Track**. ⧉ on the deck's Mixer tab detaches
the mixer into its own window.

### 6. Deliver: check, export and report

| Step | Buttons | Function |
|---|---|---|
| Check the project | **Run QA** (findings list: click to select the item) | `qa.run` |
| Fix media | Relink | `relink.run` |
| Export | Profile (Pro Tools, Resolve, …), range (all / scene / TC range), options: AAF, OTIO, print plugins, include muted tracks; **Export** | `export.run` |
| Reports | Summary, cue sheet, "active at" TC; PDF / CSV / JSON | `report.summary`, `report.write`, `report.active_at` |
| Save | Save (also Ctrl/⌘ S) | `project.save` |

Default panels: center tab **QA & Export**, picture hidden, bottom deck collapsed.

### Always available

| Buttons | Function |
|---|---|
| Undo / Redo (Ctrl/⌘ Z, Shift+Z / Y) | `undo`, `redo` |
| Save, Close | `project.save`, `project.close` |
| Play/Stop, Stop, Locate, Step, Loop | `transport.*` |
| Log tab | `log.get` |
| File pickers | `fs.list` (browser mode) |
| App info / version | `app.info` |

## Regions in detail

### Top deck (80 px)

1. **Nameplate (260 px).** The app mark, then the project name in Inter 600 14 px. A dirty dot (•)
   shows unsaved changes, and ☰ opens the project menu (New, Open, Save, Settings, Imports,
   Relink, Devices & Preferences, Plugins, Reset window layout, Close).
2. **Transport.** Keys of 40 × 36 px:
   - ⏮ to start;
   - ▶/■ play/stop, lit while playing;
   - ⏭ to end;
   - ⟲ loop, lit when on;
   - ● record, filled `--rec` while recording;
   - ◀ ▶ frame step.
3. **Counter.** The main readout is 240 px of timecode in JetBrains Mono 30 px; click to type a TC.
   Under it sit three small readouts: feet+frames, the selection's in/out, and its duration. The
   frame rate (e.g. "24 fps", "29.97 DF") sits in a pill.
4. **Session readouts.** Picture version badge (click to switch versions), sample rate,
   loop in/out.
5. **Master meters.** Horizontal L/R meters, 30 segments, peak hold 1.5 s, with the dBFS peak in a
   readout.

### Toolbar (52 px)

Grouped keys, 28 px tall. Each group has a 12 px label above it and a hairline divider between
groups; with 4 px padding above and below, the bar is 52 px.
- **Stage:** the stage bar (see Workflow).
- **Tools:** Select, Trim, Fade, Split (Scissors), Pencil (automation), Hand.
- **Snap:** Snap to frames on/off (Alt while dragging disables it), Grid readout ("1 fr").
- **Spot:** Spot mode key with a red LED when on, and Bindings.
- **Zoom:** −, +, Fit, Zoom to selection; Ctrl/⌘/Alt + wheel over the timeline.
- **History:** Undo, Redo.
- **Right edge:** Mixer (opens the deck's Mixer tab), Make cues, Populate, and Export (filled
  with `--accent-fill`).

### Left rack (264 px)

| Tab | Contents | Functions |
|---|---|---|
| Scenes | Scene list (number, title, TC range, cue count), script lines and breakdown items under each; click locates | `scene.add`, `scene.update`, `script.import` |
| Identities | Tree by category; each identity shows its assigned sound and cue count; "show all of this identity's cues" filter | `identity.add`, `identity.assign` |
| Library | Search field with category and pack filters; results with waveform thumbnails, duration, rate; ▶ audition; drag onto a cue or track | `library.search`, `audition`, `asset.peaks`, `library.import` |
| Markers | Marker lanes (show/hide eye, color, order), then markers by scene; uncued markers flagged | `marker.*`, `cues.build` |

### Inspector (300 px)

One panel at a time, chosen by the selection. The title row shows what's selected and a kind
badge.

| Selection | Sections | Functions |
|---|---|---|
| Cue | Code + category + kind badges · Description · In / Out (typeable TC) · Sound identity · Status, Priority · Owner · Notes · Marker color swatch · Sound: placeholder, scene override, replace / revert · Clips list with history · Takes · AI proposals · Perform | `cue.update`, `cue.replace`, `cue.revert`, `marker.color`, `take.add`, `ai.*`, `perform.select` |
| Marker | Lane · TC range · taps · color swatch / *Use lane's* · "Has a cue" or **Make cues from markers** · Delete | `marker.update`, `marker.color`, `cues.build`, `marker.delete` |
| Clip | Asset · position, source offset, length · gain · fades · fill · mute · Split at playhead · Delete · history | `clip.update`, `clip.split`, `clip.delete` |
| Track | Name · color · folder path · bus · gain · pan · mute / solo · inserts rack · automation lanes shown | `track.update`, `insert.*`, `automation.mode` |
| Folder | Path (rename/move) · track and clip counts · Mute all / Solo all · Ungroup | `folder.*` |
| Bus | Level · mute · balance · inserts · automation | `bus.update`, `insert.*` |
| Nothing | Project summary: scenes, cues by status, missing media, last export | `report.summary` |

### Bottom deck (per stage: 132–300 px, or collapsed to 28)

| Tab | Contents |
|---|---|
| Pads | 8 performance pads (J K L ; U I O P) for the cue being performed. Each shows its sound and category color, flashes on trigger, and has an accent ring when Shift is held. In Spot stage the pads show the spot keys (F P G B D N M) with their categories. |
| Library browser | The selected sound's large waveform with a playhead, Audition / Stop, Loop, and metadata (license, pack, rate, channels). |
| Mixer | The app's only mixer. Strips: color plate, name, insert LEDs, pan knob, a 160 px fader with dB scale, stereo meter, dB readout, M / S, and an R/T/L/W automation mode key. Bus strips come next, then a wider master strip outlined in accent. Scrolls sideways. ⧉ on the tab detaches it into its own window. |
| Log | Engine log with filters, for support. |

### Status strip (20 px)

Sample rate · frame rate · CPU % · RAM · disk streaming load · audio device (click opens
Devices) · MIDI-in LED (green flash on input) · "Saved 12 s ago" · update pill when one is
available. Everything is set in JetBrains Mono 11 px.

## Components

Build these first; every region is assembled from them.

| Component | Size | Default | Active / lit | Notes |
|---|---|---|---|---|
| Key | 28 px tall (toolbar); transport 40 × 36 | `--surface-2`, hairline, 8 px radius | Accent tint + inner ring + soft glow, bright legend. Exceptions: Mute, Solo and Record fill solid (`--mute`, `--solo`, `--rec`) with a dark legend, so they never look like a lit accent key | A real `<button>`; `aria-pressed` for toggles; optional LED dot |
| LED | 6 px | Dark dot | Color + glow | Accent by default; green for MIDI in; red for record and spot |
| Readout | Any | Deep black, inner shadow, `--readout` text with glow | — | Tabular numbers; editable readouts show a caret on click |
| Knob | 26–30 px | Flat dark cap, accent arc showing the value from −135° to +135° | — | Drag vertically (Shift: fine); double-click resets; value readout below |
| Fader | 6 px slot, 120–150 px throw | Cap 28 × 14 px | Accent cap edge while touched or writing automation | dB scale +6 … −∞ |
| Meter | Vertical 6 px (12 segments) / horizontal (30) | Unlit segments | `--ok` → `--caution` → `--error`, peak hold | Falls about 20 dB/s |
| Pill | 20 px | Dim | — | Rates, status, counts |
| Swatch picker | 14 px | The color | — | Hidden `<input type="color">`; Alt-click resets |
| Marker glyph | Lane height | Category or own color | Selected: accent outline + glow | Point, range, repeated (with taps) |
| Clip | Lane height | Category tint 18 %, waveform in the track color, name strip | Selected: accent outline + glow | Placeholder: dashed `--placeholder` outline. Muted: 40 % grey. Recording: `--rec` with a diagonal hatch, growing to the playhead. AI proposal pending: dashed white ring outside the clip + "AI" badge. Fade handles at the top corners |
| Automation lane | 44 px | Line plus breakpoints in the lane color | Writing: line in `--rec` | Click on the line keeps its value |
| Stage bar | 28 px tall, in the toolbar | Six segments | Current lit in accent; finished stages show a green (`--ok`) LED | Keyboard reachable; 1–6 select a stage |

### Track header (232 px)

1. A 4 px color stripe on the left edge: the track's own color, else its category's.
2. Row 1: the track number readout, then the name in Inter 600 13 px, with the bus name under it
   in `--text-dim`.
3. Row 2:
   - M / S keys (22 × 20 px), filled `--mute` / `--solo` when on;
   - **A**, which shows the automation lanes;
   - the R/T/L/W mode key;
   - Volume and Pan knobs.
4. A vertical meter on the right, in a 12 px well.
5. Folder rows: a ▸/▾ chevron, the folder name, M / S for everything inside, and a clip count when
   collapsed.

## Interaction and state

One app-wide store drives every panel (as now: `ui/src/store.ts`). A change anywhere updates the
selection, the inspector, the timeline and the counters together.

- `stage`: Prep, Spot, Build, Perform, Mix or Deliver.
- `transport`: playing, recording, position (samples), loop range, open spots (from
  `transport.state`).
- `selection`: kind (cue, marker, clip, track, folder or bus) plus id.
- `viewport`: start sample, samples per pixel, vertical scroll.
- `snap`, `spotMode`, `performCue`.
- `centerTab`, `deckTab`, `rackTab`.
- `theme`: accent, readout, density.

Rules:
- Selecting a cue also highlights its marker and clips, and the reverse.
- Locating moves the picture, the playhead and the counter in the same frame.
- Every edit is one undo step. Long gestures (fader moves, drags, automation passes) commit on
  release.
- Imports end with their summary sheet. Notes are listed there, never as silent drops.
- Errors appear as a toast with the engine's message, plus an entry in the Log tab.

## Keyboard shortcuts

These already exist and must keep working. The key legends must match.

| Key | Action |
|---|---|
| Space | Play / stop |
| ← / → (Shift: 1 s) | Step one frame |
| Spot mode: F, P, G, B, D, N, M | Footsteps (repeated), prop, hit, ambience range, design, note, music |
| Spot mode: Shift+key / Enter | Close an open range or footsteps run |
| Perform pads: J K L ; U I O P (Shift = accent) | Trigger pads while picture plays |
| Ctrl/⌘ Z, Shift+Z / Y, S | Undo, redo, save |
| Ctrl/⌘/Alt + wheel, Shift + wheel | Zoom, scroll timeline |
| Drag in a lane / double-click a lane | New range / point marker |
| Alt while dragging | Disable frame snapping |
| Shift-drag in the ruler | Loop range |

New in this design:
- **1–6** select a stage. Only outside text fields, and never while Spot mode or Perform owns the
  keys.
- **Ctrl/⌘ M** opens the deck's Mixer tab; again to detach it.
- **P** docks the picture or makes it picture-in-picture (outside text fields).

## Build notes

- **Draw the timeline on canvas,** as now. Only draw what's in or near the viewport, since a
  feature has thousands of markers and clips.
- **Glass needs a backdrop, and costs GPU.** Paint `--chassis-grid` and `--chassis-bg` on the
  body, and blur panels only, never the timeline canvas or meters (lanes use the opaque
  `--surface-0`). "Reduce transparency" (`data-transparency="reduced"` or the OS setting) swaps in
  opaque panels.
- **Accessibility:**
  - use real buttons and labels;
  - toggles use `aria-pressed`, and icon-only keys get an `aria-label`;
  - focus rings in accent;
  - every function reachable by keyboard or ☰;
  - contrast of at least 4.5:1 for text.
- **Respect "Reduce motion":** glows stay, but transitions drop to 0 ms.

## Acceptance checklist

- [ ] At 1680 × 1050 the five bands and three columns match the layout table within 4 px.
- [ ] Every color, font, radius and shadow comes from `tokens.css`; nothing derived from the
      accent is hard-coded.
- [ ] Switching accent, readout and density at runtime changes lit keys, glows, selection, the
      primary button and the backdrop, without a reload.
- [ ] Mute, Solo and lit keys are distinguishable side by side; a recording region, an AI
      proposal and a placeholder are distinguishable on every category color.
- [ ] Each stage opens with its default layout from the height budget table; at 1050 px Mix shows
      at least 7 tracks with the mixer open.
- [ ] There is one mixer (bottom deck), and it detaches and comes back.
- [ ] Every engine function in the Workflow tables is reachable from a visible control or ☰, and
      every control calls the function listed.
- [ ] The stage bar reconfigures the panels per stage and shows finished stages; no data changes.
- [ ] Live spotting works while playing: taps land on the right frame, open ranges show in the
      banner, Enter closes them.
- [ ] Marker lanes draw points, ranges and repeated events, with own colors and the uncued style.
- [ ] Selecting a cue highlights its marker and clips, updates the inspector, and locates on
      double-click.
- [ ] Clip move, trim, fade and split work with frame snap, and Alt disables it.
- [ ] Automation reads and writes in Read, Touch, Latch and Write from header, inspector and mixer.
- [ ] Import AAF / OTIO show the summary sheet, including cues restored and scenes added.
- [ ] Export writes the chosen files and lists them, with QA findings linked to their items.
- [ ] Under 1280 px nothing overlaps; the inspector becomes an overlay and the rack an icon rail.
- [ ] All shortcuts in the table work.
