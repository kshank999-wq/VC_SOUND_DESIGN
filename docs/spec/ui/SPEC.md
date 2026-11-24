# VC Sound Design — UI Spec v2.1

Nov 24, 2026 · v2.1: v2 (Nov 23, revised after the design review) plus measured color and
contrast fixes. Supersedes the v1 brief and the earlier v2 drafts.
Approved mockups are in `mockups/` (see README.md). Where this text and the mockups disagree on
look, the mockups win; where they disagree on behavior or functions, this text wins.

## How to use this file

You are designing the UI of **VC Sound Design**, a picture-locked sound-design and Foley DAW.
This one file has everything you need:

1. **The spec** (next section): layout, screens, the six-stage workflow with every button and the
   function it calls, regions, components, state, shortcuts, and an acceptance checklist.
2. **Appendix A — design tokens** (`tokens.css`): every color, font, radius, shadow and size. Use
   these values; don't invent one-offs.
3. **Appendix B — VC Composer 2 layout reference**: the sister DAW whose layout this app shares.
   Match its structure, with the more modern skin described in the spec.
4. **Appendix C — the full function list**: every command the app's engine exposes. Every control
   you design calls one of these; design no controls that need anything else.

**Delivered (v2):** mockups of the Edit window at 1680 × 1050, one per stage (Prep, Spot,
Build, Perform, Mix, Deliver), in `mockups/`. The stage bar switches between them. Use realistic placeholder
content: a short film, "Gravel Road", reel 1 at 24 fps from 01:00:00:00, with two scenes and about
twelve cues across the default lanes. Then add the key secondary screens: the Import summary,
QA & Export, Devices & Preferences, and the Welcome screen.

Paths like `ui/src/…` or `crates/…` refer to the app's repository. They are context for the
engineers who build your design; you don't need them.

## Changes in v2

These override anything later in this file that conflicts.

1. **Tokens (Appendix A, replaced).**
   - Accent-derived values (`--accent-glow`, `--accent-soft`, the lit-key gradient) are computed
     from `--accent` with `color-mix()`, so the accent theme switches everything at runtime.
   - Every status and category color is unique:
     - Hard FX is orange `#FB923C` (it was the same red as `--rec`).
     - Backgrounds are teal `#2DD4BF` (they were close to the accent blue).
     - Design FX is violet `#A78BFA` (it was the same value as `--ai`).
     - Dialogue notes are neutral `#E2E8F0` (they were the same yellow as solo/warn).
     - Music is pink `#F472B6`.
     - Mute is a neutral fill with a dark legend (it was a blue close to the accent). v2.1 sets it to
       zinc `#A1A1AA`.
     - AI is cyan `#22D3EE` with a dashed ring. v2.1 removes cyan as an accent option, so AI no
       longer needs a substitute color.
   - `--text-faint` passes 4.5:1. v2.1 sets it to `#949EB0`, which passes on every surface.
   - The chassis carries a faint accent glow and a 28 px grid (`--chassis-fx`), so the panel blur
     reads as glass.
2. **Vertical space: per-stage layout defaults.** At 1050 px tall one fixed split leaves room for
   only 2–3 tracks. Each stage now has its own default picture height, marker-lane mode, open
   folders and bottom-deck height. Users can still drag any split, and `layout.set` persists it.

   | Stage | Picture | Marker lanes | Open folders | Bottom deck | Left rack | Inspector |
   |---|---|---|---|---|---|---|
   | Prep | 300 px | folded (10 px strips) | none | Library browser, 200 px | Scenes | Project + prep checklist |
   | Spot | 320 px | full (26 px, labels + key) | none (collapsed rows) | Pads (spot keys), 150 px | Markers | Marker |
   | Build | 170 px | folded | FOLEY, FX | Library browser, 200 px | Library | Cue (+ AI proposals) |
   | Perform | 300 px | folded | FOLEY | Pads (perform), 170 px | Identities | Cue: perform + takes |
   | Mix | hidden | hidden (one 18 px "7 folded" row) | FOLEY, FX + automation + bus rows | Mixer, 250 px | Scenes | Track |
   | Deliver | hidden | — | — | collapsed to 28 px (Log tab) | Scenes | Project + last export |

   Deliver replaces the timeline with the **QA & Export** center tab: findings · export · reports.
3. **Folded marker lanes.** Outside Spot, the seven marker lanes become 10 px strips: a minimap
   with no labels, and the selection is still outlined in accent.
4. **Sizes.**
   - Track lane default 56 px (tall 88, compact 48). Folder rows 22 px, bus rows 22 px.
   - Toolbar keys are 22 px tall so label + key fit the 44 px toolbar.
   - Page padding 8 px, panel gap 6 px.
5. **One mixer placement.** The mixer lives in the bottom deck (Mix stage default) and detaches
   with *Mixer ⧉* / Ctrl/⌘ M. The center "Mixer" tab opens the same detached window; it doesn't
   keep a third copy.
6. **CSS gotcha found in the mockups.** Any button reset must have zero specificity
   (`:where(.app) button { font: inherit }`). Otherwise it overrides `.key` and every key label
   renders at body size.

## Changes in v2.1

Measured fixes to v2's colors. Two colors that share a surface must be told apart; this is
measured as a CIEDE2000 difference (ΔE: about 2 is barely visible, 10 is clearly different, 20+
is unmistakable). Contrast is WCAG. These override v2's values above and in the mockups. The
mockups and screenshots in this package are updated.

| v2 value | Problem (measured) | v2.1 |
|---|---|---|
| Accent options cyan, violet, amber | Violet = Design FX (ΔE 0); amber ≈ Solo (7.0); cyan = the AI ring | Options are **blue** `#5B8CFF` (default), **indigo** `#6366F1` and **magenta** `#C026D3`, set with `data-accent`. Each has its own `--on-accent` legend (≥ 4.5:1) |
| `--mute` `#CBD5E1` | ≈ Dialogue stripe beside it in the track header (4.9) | Zinc `#A1A1AA`, with a dark legend (7.4:1) |
| `--playhead` = the mint readout | Lost on Backgrounds clips (8.9) | Pink-lilac `#F0ABFC` with `--playhead-glow`, at least 15.6 from every category |
| `--solo` = `--warn` = `--meter-amber` `#FBBF24` | One value with three meanings; amber meter segments sit next to the S key in every strip | Solo keeps `#FBBF24`. Warn and amber meter segments become lemon `#FEF08A` (15.7 from Solo) |
| `--ok` / `--meter-green` `#34D399` | ≈ Footsteps (7.3) | `#16A34A` (16.0) |
| `--error` / `--meter-red` `#F87171` | ≈ Record (9.3) | `#DC2626` (12.9) for icons and meter segments. It's 3.7:1, so text uses `--error-text` `#F87171` (≥ 4.8:1) |
| "passed" text in the meter green | Would equal Footsteps | `--ok-text` `#86EFAC` |
| `--text-dim` `#8A94A6`, `--text-faint` `#7B8597` | 4.37:1 on hover; faint 4.27:1 on keys and 3.59:1 on hover | `#9AA4B5` (≥ 5.3:1) and `#949EB0` (≥ 4.9:1) on every surface |

**Rules and the measured minimum for each:**

| Rule | Minimum | Measured (closest pair) |
|---|---|---|
| Categories against each other | ΔE 15 | 15.8 (Footsteps / Props) |
| Every accent option against every category | ΔE 14 | 14.4 (blue / Design FX) |
| Every accent option against Mute and Solo | ΔE 20 | 24.6 (blue / Mute) |
| Record against every category | ΔE 15 | 18.6 (Music) |
| AI ring against every category and accent | ΔE 15 | 16.2 (Backgrounds) |
| Playhead against every category | ΔE 15 | 15.6 (Music) |
| Mute, Solo and Record against each other | ΔE 20 | 30.4 (Mute / Record) |
| Mute and Solo keys against the track stripes | ΔE 10 | 17.9 (Mute / Dialogue) |
| Meter amber against Solo | ΔE 15 | 15.7 |
| Meter green against Footsteps | ΔE 15 | 16.0 |
| Error against Record | ΔE 10 | 12.9 |
| Text tokens on every surface (0–3, hover included) | 4.5:1 | 4.83:1 (`--error-text`) |
| Dark legend on every category, Mute and Solo fill | 4.5:1 | 7.4:1 |

## The spec

Nov 23, 2026

### Overview

Redesign the VC Sound Design desktop app: a picture-locked sound-design and Foley DAW where
editors spot events against picture, turn them into cues, build tracks from a sound library,
perform and record, mix, and deliver to editorial (AAF, OTIO, stems, cue sheets).

**Layout:** use the same structure as the VC Composer 2 DAW. That means:
- a top deck of transport and readouts;
- a toolbar of grouped keys;
- three columns: left rack, edit window and inspector;
- a tabbed bottom deck;
- a thin status strip.

Appendix B summarizes what you need from VC Composer 2's own UI spec. Reuse its component vocabulary: panels, keycaps with LEDs, backlit readouts, knobs, meters
and a stage bar.

**Look:** a more modern tech version of that hardware console:
- glass panels over a near-black chassis, with hairline borders instead of screws and bevels;
- crisp monospace readouts with a soft glow;
- one accent color, and motion that feels instant.

The rest of this spec is the source of truth for structure, functions, sizes and behavior.
Appendix A (`tokens.css`) is the source of truth for colors, type and depth.

- **Desktop first:** reference window 1680 × 1050, minimum 1280 × 800. Like VC Composer, it is a
  dense pro tool, so controls are smaller than 44 px touch targets on purpose. A detachable mixer
  window serves a second monitor.
- **Same app, new skin and structure:** every function listed here already exists as an engine
  command (`crates/vcsd-engine/src/lib.rs`), called from the UI through `call(cmd, args)` in
  `ui/src/api.ts`. The current UI (`ui/src/`) shows the arguments each one takes. Design for these
  functions; don't invent new back-end behavior.
- **Placeholder content for mockups:** a short film, "Gravel Road", reel 1 at 24 fps from
  01:00:00:00, with two scenes and about twelve cues across the default lanes. Render real project
  data in the build.

### What carries over from VC Composer, and what's more modern

| | VC Composer 2 | VC Sound Design |
|---|---|---|
| Bands | Top deck · toolbar · rack / edit / inspector · keyboard deck · status | Same five bands |
| Workspace switch | Composer \| Producer on the nameplate | **Stage bar**: Prep › Spot › Build › Perform › Mix › Deliver (one data model; stages change tools, not data) |
| Edit lanes | Notation staves or MIDI/audio regions | Picture viewer above a **marker timeline** (spotting lanes) above **audio tracks** with automation lanes |
| Bottom deck | 88-key keyboard / mixer | **Pads / Library / Mixer** tabs |
| Panels | Anodized gradient, screws, bevel | Glass (`--surface-1` + 12 px backdrop blur), 1 px hairline, a 1 px top sheen; no screws |
| Keys | Raised keycap, 2 px drop, LED glow when lit | Flat rounded key (8 px radius), 1 px hairline; lit = accent inner ring + soft glow; optional 6 px LED dot |
| Readouts | LCD with a retro mono face | Deep-black readout, JetBrains Mono with tabular figures, soft glow in `--readout` |
| Accent | Amber `--led` | Electric blue `--accent` (themeable: blue, indigo, magenta) |
| Motion | Mostly static | 120–200 ms ease-out on hover/press/panel changes; playhead and meters at display refresh |

### Design tokens

Use Appendix A (`tokens.css`) before designing or building any component. No hard-coded colors, radii, fonts or
shadows outside it.

- **Themeable at runtime** (Preferences): `--accent` (blue / indigo / magenta, via `data-accent`), `--readout`
  (mint / green / amber / ice), and density (`comfortable` / `compact`).
- **Category colors** (`--cat-*`) color marker lanes, cue badges, track stripes and clip tints.
  A lane, marker or track can override them with its own color; the UI already supports this.
- **Fonts:** Barlow Condensed for labels and key legends (uppercase, tracked), Inter for UI text,
  JetBrains Mono for every number that changes (timecode, dB, counters). All are free on Google
  Fonts.

### Layout

Five horizontal bands. Sizes are at the 1680 px reference width.

| # | Region | Size | Contents (left to right) |
|---|---|---|---|
| 1 | Top deck | 84 px | Nameplate (project name, dirty dot, ☰ project menu) · Transport · Timecode counter · Session readouts · Master meters |
| 2 | Toolbar | 44 px (keys 22 px) | Stage bar · Edit tools · Snap/Grid · Spot mode · Zoom · Undo/Redo · Export |
| 3a | Left rack | 264 px, resizable 200–420 | Tabs: **Scenes** (script breakdown) · **Identities** · **Library** · **Markers** |
| 3b | Edit window | Fills the rest (min 640) | Picture viewer (upper, resizable split) over the **timeline**: ruler, marker lanes, track lanes, bus lanes. Center tabs: Timeline · Cue chart · Mixer · QA & Export |
| 3c | Inspector | 300 px, resizable 240–460 | Context panel for the selection: cue, marker, clip, track, folder, bus, or project |
| 4 | Bottom deck | Per stage, 150–250 px (collapsible to 28); see Changes in v2 | Tabs: **Pads** (perform) · **Library browser** (waveform audition) · **Mixer** (strips) · **Log** |
| 5 | Status strip | 20 px | Sample rate · frame rate · CPU / RAM / disk streaming · audio device · MIDI in LED · autosave time · update pill |

Panel edges between 3a / 3b / 3c and the picture/timeline split are draggable; double-click
restores the default (`layout.get` / `layout.set` persist it per project). ☰ → *Reset window
layout* restores everything.

Under 1280 px wide, the inspector becomes an overlay sheet (toggle key in the toolbar), and the
left rack collapses to an icon rail.

#### Edit window internals

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
  - **Heights:** 56 px by default, 88 px tall, 48 px compact. Folder rows 22 px.
  - **Folders:** rows are grouped in collapsible folders. A collapsed folder shows all its clips on
    one row.
  - **Automation lanes:** each track can show its own (Volume, Pan, and any plugin parameter), 44 px
    each, under the track.
- **Bus section** below the tracks: stem buses and the master, with their own automation lanes.
- **Playhead:** a 1.5 px `--playhead` (pink-lilac, with `--playhead-glow`) line through the ruler and every lane, with a cap in the
  timecode row.
- **Pinned and scrolling:** track headers stay pinned on horizontal scroll, and the ruler scrolls
  with the lanes.

### Screens

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
| **Mixer** | Bottom deck or detached window | Toolbar *Mixer ⧉*, center tab (opens the detached window) | One strip per track and bus, master strip outlined in accent |
| **QA & Export** | Center tab | Toolbar *Export*, Deliver stage | QA run with findings; export profile and options; output file list; reports |
| Plugin manager | Dialog | ☰, insert slot "+" | Scan folders, list VST3/CLAP/AU, add to slot |
| Plugin editor | Floating window | Double-click an insert | The plugin's own editor or a generic parameter list |
| Devices & Preferences | Dialog | ☰, status strip device name | Audio device, buffer, MIDI inputs, test tone; theme (accent, readout, density); spot key bindings |
| Spot bindings | Sheet in Preferences | Spot mode key long-press | Map keys and MIDI notes to categories and marker kinds |
| AI proposals | Inspector section + review sheet | Cue inspector, Build stage | Run AI, then accept or reject each proposal |
| Reports | Sheet | Deliver stage | Cue sheet / summary as PDF, CSV, JSON; "active at" report |
| Updates | Sheet | Status strip pill | Release notes, install |

### Workflow (the stage bar)

The stage bar sits in the toolbar's left group: **Prep › Spot › Build › Perform › Mix › Deliver**.
- The current stage is lit in accent.
- Finished stages show a green LED. A stage counts as done when its checklist is met, for example
  "every marker is a cue" for Spot.
- Clicking a stage reconfigures the toolbar, the left rack's default tab, the inspector's default
  section and the bottom deck's default tab.
- No data changes when switching stages, and every function stays reachable from any stage through
  ☰ or the context menus.

#### 1. Prep: set up the project and bring material in

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

Default panels: left rack **Scenes**, bottom deck **Library**.

#### 2. Spot: mark every sound event against picture

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

Default panels: left rack **Markers**, inspector **Marker**, bottom deck **Pads** (spot keys
shown as pads with category colors). While Spot mode is on, the toolbar shows a banner, "SPOT —
open: F, B (Enter closes)", in accent.

#### 3. Build: put sounds on the cues

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

Default panels: left rack **Library**, inspector **Cue**, bottom deck **Library browser**.

#### 4. Perform: record Foley and performed effects

| Step | Buttons | Function |
|---|---|---|
| Choose the cue to perform | **Perform** in the cue inspector | `perform.select`, `perform.start` |
| Trigger sounds live | Pads J K L ; U I O P (Shift = accent), MIDI pads | `perform.trigger`, `midi.message` |
| Keep or drop the pass | **Commit** / **Cancel** | `perform.commit`, `perform.cancel` |
| Record a microphone | **Record** (red), Stop | `record.start`, `record.stop` |
| Add a take from a file | Add take | `take.add` |
| Loop a section | Loop key, Shift-drag on the ruler | `transport.loop` |

Default panels: bottom deck **Pads**, inspector **Cue** (takes list).

#### 5. Mix: balance, automate and process

| Step | Buttons | Function |
|---|---|---|
| Track level, pan, mute, solo, color, folder, bus, height | Header knobs / keys; inspector fields; header swatch (Alt-click resets) | `track.update` |
| Folders | Move, rename, ungroup; folder M / S | `folder.move`, `folder.rename`, `folder.ungroup`, `folder.flag` |
| Buses | Level, mute, balance (bus strips) | `bus.update` |
| Automation | **A** key shows lanes; R / T / L / W mode key; click to add a point; drag to move; double-click to delete; Clear lane | `automation.set`, `automation.update`, `automation.delete`, `automation.clear`, `automation.mode`, `automation.touch` |
| Inserts | Slot "+" (plugin list), bypass LED, drag to reorder, remove | `plugins.list`, `plugins.scan`, `plugins.folders`, `insert.add`, `insert.move`, `insert.bypass`, `insert.remove` |
| Plugin parameters | Plugin editor knobs, automate a parameter | `plugin.params`, `plugin.param` |

Default panels: center tab **Timeline**, bottom deck **Mixer**, inspector **Track**. *Mixer ⧉*
detaches the mixer into its own window.

#### 6. Deliver: check, export and report

| Step | Buttons | Function |
|---|---|---|
| Check the project | **Run QA** (findings list: click to select the item) | `qa.run` |
| Fix media | Relink | `relink.run` |
| Export | Profile (Pro Tools, Resolve, …), range (all / scene / TC range), options: AAF, OTIO, print plugins, include muted tracks; **Export** | `export.run` |
| Reports | Summary, cue sheet, "active at" TC; PDF / CSV / JSON | `report.summary`, `report.write`, `report.active_at` |
| Save | Save (also Ctrl/⌘ S) | `project.save` |

Default panels: center tab **QA & Export**.

#### Always available

| Buttons | Function |
|---|---|
| Undo / Redo (Ctrl/⌘ Z, Shift+Z / Y) | `undo`, `redo` |
| Save, Close | `project.save`, `project.close` |
| Play/Stop, Stop, Locate, Step, Loop | `transport.*` |
| Log tab | `log.get` |
| File pickers | `fs.list` (browser mode) |
| App info / version | `app.info` |

### Regions in detail

#### Top deck (84 px)

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

#### Toolbar (44 px)

Grouped keys. Each group has a small label above it and a hairline divider between groups.
- **Stage:** the stage bar (see Workflow).
- **Tools:** Select, Trim, Fade, Split (Scissors), Pencil (automation), Hand.
- **Snap:** Snap to frames on/off (Alt while dragging disables it), Grid readout ("1 fr").
- **Spot:** Spot mode key with a red LED when on, and Bindings.
- **Zoom:** −, +, Fit, Zoom to selection; Ctrl/⌘/Alt + wheel over the timeline.
- **History:** Undo, Redo.
- **Right edge:** Mixer ⧉, Make cues, Populate, and Export (accent-filled).

#### Left rack (264 px)

| Tab | Contents | Functions |
|---|---|---|
| Scenes | Scene list (number, title, TC range, cue count), script lines and breakdown items under each; click locates | `scene.add`, `scene.update`, `script.import` |
| Identities | Tree by category; each identity shows its assigned sound and cue count; "show all of this identity's cues" filter | `identity.add`, `identity.assign` |
| Library | Search field with category and pack filters; results with waveform thumbnails, duration, rate; ▶ audition; drag onto a cue or track | `library.search`, `audition`, `asset.peaks`, `library.import` |
| Markers | Marker lanes (show/hide eye, color, order), then markers by scene; uncued markers flagged | `marker.*`, `cues.build` |

#### Inspector (300 px)

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

#### Bottom deck (220 px)

| Tab | Contents |
|---|---|
| Pads | 8 performance pads (J K L ; U I O P) for the cue being performed. Each shows its sound and category color, flashes on trigger, and has an accent ring when Shift is held. In Spot stage the pads show the spot keys (F P G B D N M) with their categories. |
| Library browser | The selected sound's large waveform with a playhead, Audition / Stop, Loop, and metadata (license, pack, rate, channels). |
| Mixer | Strips: color plate, name, insert LEDs, pan knob, a 140 px fader with dB scale, stereo meter, dB readout, M / S, and an R/T/L/W automation mode key. Bus strips come next, then a wider master strip outlined in accent. Scrolls sideways. |
| Log | Engine log with filters, for support. |

#### Status strip (20 px)

Sample rate · frame rate · CPU % · RAM · disk streaming load · audio device (click opens
Devices) · MIDI-in LED (green flash on input) · "Saved 12 s ago" · update pill when one is
available. Everything is set in JetBrains Mono 11 px.

### Components

Build these first; every region is assembled from them.

| Component | Size | Default | Active / lit | Notes |
|---|---|---|---|---|
| Key | 22–30 px tall (toolbar 22); transport 40 × 36 | `--surface-2`, hairline, 8 px radius | Accent inner ring, soft glow, bright legend | A real `<button>`; `aria-pressed` for toggles; optional LED dot |
| LED | 6 px | Dark dot | Color + glow | Accent by default; green for MIDI in; red for record and spot |
| Readout | Any | Deep black, inner shadow, `--readout` text with glow | — | Tabular numbers; editable readouts show a caret on click |
| Knob | 26–30 px | Flat dark cap, accent arc showing the value from −135° to +135° | — | Drag vertically (Shift: fine); double-click resets; value readout below |
| Fader | 6 px slot, 120–150 px throw | Cap 28 × 14 px | Accent cap edge while touched or writing automation | dB scale +6 … −∞ |
| Meter | Vertical 6 px (12 segments) / horizontal (30) | Unlit segments | `--meter-green` → `--meter-amber` (lemon) → `--meter-red`, peak hold | Falls about 20 dB/s |
| Pill | 20 px | Dim | — | Rates, status, counts |
| Swatch picker | 14 px | The color | — | Hidden `<input type="color">`; Alt-click resets |
| Marker glyph | Lane height | Category or own color | Selected: accent outline + glow | Point, range, repeated (with taps) |
| Clip | Lane height | Category tint 18 %, waveform in the track color, name strip | Selected: accent outline | Placeholder clips are dashed; muted clips are 40 % grey; fade handles at the top corners |
| Automation lane | 44 px | Line plus breakpoints in the lane color | Writing: line in `--rec` | Click on the line keeps its value |
| Stage bar | 44 px | Six segments | Current lit; finished stages show a green LED | Keyboard reachable |

#### Track header (232 px)

1. A 4 px color stripe on the left edge: the track's own color, else its category's.
2. Row 1: the track number readout, then the name in Inter 600 13 px, with the bus name under it
   in `--text-dim`.
3. Row 2:
   - M / S keys (22 × 20 px), filled `--mute` (zinc, dark legend) / `--solo` when on;
   - **A**, which shows the automation lanes;
   - the R/T/L/W mode key;
   - Volume and Pan knobs.
4. A vertical meter on the right, in a 12 px well.
5. Folder rows: a ▸/▾ chevron, the folder name, M / S for everything inside, and a clip count when
   collapsed.

### Interaction and state

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

### Keyboard shortcuts

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
- **Ctrl/⌘ M** opens or detaches the mixer.

### Build notes

- **Draw the timeline on canvas,** as now. Only draw what's in or near the viewport, since a
  feature has thousands of markers and clips.
- **Glass costs GPU.** Apply backdrop blur to the panels only, never to the timeline canvas or
  meters. Provide a "Reduce transparency" preference that swaps `--surface-1` for an opaque color.
- **Accessibility:**
  - use real buttons and labels;
  - toggles use `aria-pressed`, and icon-only keys get an `aria-label`;
  - focus rings in accent;
  - every function reachable by keyboard or ☰;
  - contrast of at least 4.5:1 for text.
- **Respect "Reduce motion":** glows stay, but transitions drop to 0 ms.

### Acceptance checklist

- [ ] At 1680 × 1050 the five bands and three columns match the layout table within 4 px.
- [ ] Every color, font, radius and shadow comes from `tokens.css`.
- [ ] Accent, readout and density switch at runtime without a reload.
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
- [ ] The mixer detaches to a second window and stays in sync.
- [ ] Under 1280 px nothing overlaps; the inspector becomes an overlay and the rack an icon rail.
- [ ] All shortcuts in the table work.

## Appendix A — Design tokens (`tokens.css`, v2.1)

Same file as `tokens.css` in this package. Import it before any component.

```css
/* VC Sound Design — design tokens v2 ("modern console")
   Fixes from the v1 review (Nov 23, 2026):
   1. Accent-derived values are computed from --accent, so the accent theme switches everything.
   2. No two status / category colors share a value; mute no longer reads as a lit key.
   3. --text-faint passes 4.5:1 on panels and lanes.
   4. The chassis carries a faint glow + grid so the panel blur has something to blur.
   5. Stage-specific layout defaults (see the bottom block) replace the single fixed split.
   v2.1 (Nov 24, 2026): measured fixes. Every pair of colors that shares a surface is at least
   ΔE2000 15 apart (status keys 20), and text passes 4.5:1 on every surface. See SPEC.md ›
   Changes in v2.1 for the table. Accent options are limited to ones that pass. */

:root {
  /* ---- Themeable ---- */
  --accent: #5B8CFF;                     /* blue (default); themes: [data-accent] at the bottom */
  --on-accent: #05070A;                  /* legend on a filled accent key */
  --accent-glow: color-mix(in srgb, var(--accent) 45%, transparent);
  --accent-soft: color-mix(in srgb, var(--accent) 14%, transparent);
  --accent-lit-top: color-mix(in srgb, var(--accent) 24%, transparent);
  --accent-lit-bottom: color-mix(in srgb, var(--accent) 8%, transparent);
  --readout: #7CF5D8;                    /* options: #9BF07A green, #FFC266 amber, #C7D2FE ice */
  --readout-glow: color-mix(in srgb, var(--readout) 35%, transparent);

  /* ---- Surfaces ---- */
  --chassis: #07080B;
  --chassis-fx:
    radial-gradient(900px 520px at 18% -8%, color-mix(in srgb, var(--accent) 13%, transparent), transparent 70%),
    radial-gradient(800px 480px at 102% 108%, rgba(45, 212, 191, .07), transparent 70%),
    repeating-linear-gradient(0deg, rgba(255,255,255,.018) 0 1px, transparent 1px 28px),
    repeating-linear-gradient(90deg, rgba(255,255,255,.018) 0 1px, transparent 1px 28px),
    var(--chassis);
  --surface-0: #0D0F14;
  --surface-1: rgba(22, 25, 33, .72);    /* panels, over 14px blur + 140% saturate */
  --surface-1-opaque: #161921;           /* "Reduce transparency" */
  --surface-2: rgba(32, 36, 47, .9);
  --surface-3: #2A2F3D;
  --hairline: rgba(255, 255, 255, .07);
  --hairline-strong: rgba(255, 255, 255, .13);
  --edge-light: linear-gradient(90deg, transparent, rgba(255,255,255,.18), transparent);
  --readout-bg: #04070A;

  /* ---- Text ---- */
  --text: #D6DCE6;
  --text-bright: #F4F7FB;
  --text-dim: #9AA4B5;                   /* labels; ≥5.3:1 on every surface, hover included */
  --text-faint: #949EB0;                 /* ≥4.9:1 on every surface (v2's #7B8597 failed on keys and hover) */

  /* ---- Status (each value unique) ---- */
  --meter-green: #16A34A;                /* = --ok; ΔE 16 from Footsteps (v2's #34D399 was 7) */
  --meter-amber: #FEF08A;                /* = --warn; lemon, ΔE 15.7 from Solo (v2 used Solo's value) */
  --meter-red: #DC2626;                  /* = --error; ΔE 12.9 from --rec */
  --rec: #F43F5E;
  --solo: #FBBF24;
  --mute: #A1A1AA;                       /* zinc + dark legend; ΔE 17.9 from Dialogue (v2's #CBD5E1 was 4.9) */
  --ok: #16A34A;                         /* icons, meter segments */
  --warn: #FEF08A;
  --error: #DC2626;                      /* icons, meter segments only: 3.7:1, too low for text */
  --ok-text: #86EFAC;                    /* "passed" labels (≥7:1) */
  --error-text: #F87171;                 /* error labels, Delete (≥4.8:1 on every surface) */
  --playhead: #F0ABFC;                   /* ≥15.6 from every category (the mint readout was 8.9 from Backgrounds) */
  --playhead-glow: color-mix(in srgb, var(--playhead) 45%, transparent);
  --marker-flag: #FDE047;
  --ai: #22D3EE;                         /* dashed ring + badge; fixed (cyan is no longer an accent option) */
  --placeholder: #94A3B8;

  /* ---- Spotting categories (distinct hue AND lightness) ---- */
  --cat-footsteps: #4ADE80;
  --cat-props: #BEF264;
  --cat-hardfx: #FB923C;                 /* was #F43F5E (= --rec) */
  --cat-background: #2DD4BF;             /* was #60A5FA (≈ accent, mute) */
  --cat-designfx: #A78BFA;               /* was #C084FC (= --ai) */
  --cat-dialogue: #E2E8F0;               /* was #FACC15 (≈ solo/warn) */
  --cat-music: #F472B6;
  --cat-custom: #D6C7A1;

  /* ---- Type ---- */
  --font-label: 'Barlow Condensed', sans-serif;
  --font-ui: 'Inter', system-ui, sans-serif;
  --font-readout: 'JetBrains Mono', 'Share Tech Mono', monospace;

  /* ---- Shape, depth, motion ---- */
  --r-panel: 12px;
  --r-key: 8px;
  --r-readout: 6px;
  --r-pill: 999px;
  --gap-panel: 6px;
  --pad-page: 8px;
  --blur: 14px;
  --shadow-panel: 0 1px 0 rgba(255,255,255,.05) inset, 0 8px 24px rgba(0,0,0,.45);
  --shadow-key: 0 1px 0 rgba(255,255,255,.08) inset, 0 1px 2px rgba(0,0,0,.5);
  --shadow-key-on: 0 0 0 1px var(--accent) inset, 0 0 14px var(--accent-glow);
  --shadow-readout: 0 1px 6px rgba(0,0,0,.9) inset, 0 0 0 1px rgba(255,255,255,.04);
  --ease: cubic-bezier(.2, .8, .2, 1);
  --t-fast: 120ms;
  --t-med: 200ms;

  /* ---- Geometry (reference 1680 × 1050) ---- */
  --top-deck: 84px;
  --toolbar: 44px;
  --toolbar-key: 22px;                   /* label 10 + gap 3 + key 22 fits 44 */
  --left-rack: 264px;
  --inspector: 300px;
  --track-header: 232px;
  --lane-marker: 26px;
  --lane-marker-folded: 10px;            /* minimap strip outside Spot */
  --lane-track: 56px;                    /* default is now the old "compact" */
  --lane-track-tall: 88px;
  --lane-folder: 22px;
  --lane-auto: 44px;
  --lane-bus: 22px;
  --ruler: 66px;
  --status: 20px;
}

/* Stage layout defaults: picture height · marker lanes · bottom deck.
   Set data-stage on the app root; users can still drag the splits (layout.set persists them). */
[data-stage="prep"]    { --picture: 300px; --bottom-deck: 200px; }  /* lanes folded, folders collapsed */
[data-stage="spot"]    { --picture: 320px; --bottom-deck: 150px; }  /* all 7 lanes at 26px, folders collapsed */
[data-stage="build"]   { --picture: 170px; --bottom-deck: 200px; }  /* lanes folded, FOLEY + FX open */
[data-stage="perform"] { --picture: 300px; --bottom-deck: 170px; }  /* lanes folded, FOLEY open */
[data-stage="mix"]     { --picture: 0px;   --bottom-deck: 250px; }  /* picture + lanes hidden, buses shown */
[data-stage="deliver"] { --picture: 0px;   --bottom-deck: 28px;  }  /* center = QA & Export, deck collapsed */

/* Accent themes (on the app root). Each keeps ΔE ≥ 14 from every category and ≥ 20 from Mute
   and Solo. Cyan, violet and amber are not offered: cyan is the AI ring, violet equals Design
   FX, and amber is Solo. */
[data-accent="blue"]    { --accent: #5B8CFF; --on-accent: #05070A; }   /* legend 6.4:1 */
[data-accent="indigo"]  { --accent: #6366F1; --on-accent: #05070A; }   /* legend 4.5:1 */
[data-accent="magenta"] { --accent: #C026D3; --on-accent: #FFFFFF; }   /* legend 4.7:1 */

[data-density="compact"] {
  --lane-track: 48px;
  --lane-marker: 22px;
  --track-header: 208px;
}

[data-reduce-transparency] { --surface-1: var(--surface-1-opaque); --blur: 0px; }

/* Component recipes */
.app     { background: var(--chassis-fx); }
.panel   { background: var(--surface-1); backdrop-filter: blur(var(--blur)) saturate(140%); border: 1px solid var(--hairline); border-radius: var(--r-panel); box-shadow: var(--shadow-panel); position: relative; }
.panel::before { content: ""; position: absolute; left: 14px; right: 14px; top: 0; height: 1px; background: var(--edge-light); }
.label   { font: 600 10px/1 var(--font-label); letter-spacing: .14em; text-transform: uppercase; color: var(--text-dim); }
.readout { background: var(--readout-bg); border-radius: var(--r-readout); box-shadow: var(--shadow-readout); color: var(--readout); font-family: var(--font-readout); font-variant-numeric: tabular-nums; text-shadow: 0 0 8px var(--readout-glow); }
.key     { background: var(--surface-2); border: 1px solid var(--hairline-strong); border-radius: var(--r-key); box-shadow: var(--shadow-key); color: var(--text); font: 600 11.5px/1 var(--font-label); letter-spacing: .07em; text-transform: uppercase; transition: background var(--t-fast) var(--ease), box-shadow var(--t-fast) var(--ease); }
.key:hover { background: var(--surface-3); }
.key[aria-pressed="true"] { background: linear-gradient(180deg, var(--accent-lit-top), var(--accent-lit-bottom)); box-shadow: var(--shadow-key-on); color: var(--text-bright); }
.key.primary { background: var(--accent); color: var(--on-accent); border-color: transparent; box-shadow: 0 0 16px var(--accent-glow); }
.key.mute[aria-pressed="true"] { background: var(--mute); color: #05070A; box-shadow: none; }
.key.solo[aria-pressed="true"] { background: var(--solo); color: #05070A; box-shadow: none; }
.key:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.led     { width: 6px; height: 6px; border-radius: 50%; background: #262B36; }
.led.on  { background: var(--accent); box-shadow: 0 0 8px var(--accent-glow); }
.pill    { border-radius: var(--r-pill); padding: 2px 8px; font: 600 10.5px/1.4 var(--font-ui); }
.ai-proposal { border: 1px dashed var(--ai); background: color-mix(in srgb, var(--ai) 6%, transparent); }

@media (prefers-reduced-motion: reduce) { * { transition-duration: 0ms !important; } }
```

## Appendix B — VC Composer 2 layout reference

VC Sound Design shares VC Composer 2's layout and component vocabulary. These are the essentials
from VC Composer's own UI spec (`kshank999-wq/vc_composer2/docs/spec/ui/SPEC.md`). Match the
structure; the skin is the modern one in the spec and Appendix A.

### Bands (VC Composer, at 1680 px)

| # | Region | Size | Contents |
|---|---|---|---|
| 1 | Top deck | 96 px | Brand nameplate with a view switch · transport (7 keys, 46 × 40 px) · counter (main readout 250 px + start/end/length readout) · session readouts · master meters |
| 2 | Toolbar | ~58 px | Groups of keys, each with a small uppercase label above and a divider between |
| 3a | Left rack | 184 px | Tracks list (show/hide eye per track) · groups · markers (click to locate) |
| 3b | Edit window | Fills the rest | Title bar with session name and **Stage bar** · overview strip (30 px) · ruler (rows of 22 px) · track rows; scrolls horizontally inside its own box |
| 3c | Inspector | 268 px | Selection readout and context controls |
| 4 | Bottom deck | 140–236 px | 88-key keyboard, or the mixer (tab to switch) |
| 5 | Status strip | 16 px | Sample rate, CPU, disk, autosave time in a mono face |

### Conventions to keep

- **Track headers** (220 px) stay pinned when lanes scroll sideways. Each has:
  - a color stripe on the left edge;
  - a number readout and a name;
  - R / S / M keys that fill solid red, amber or blue when on;
  - volume and pan knobs;
  - a vertical meter in a dark well on the right.
- **Keys:**
  - real buttons;
  - lit state = accent ring + glow;
  - an optional 6 px LED dot left of the legend.
- **Readouts:** dark wells with glowing mono text; secondary labels at 65 % opacity.
- **Knobs:** drag vertically (Shift: fine); double-click resets; value readout below.
- **Meters:**
  - green to amber to red;
  - fall about 20 dB per second;
  - 1.5 s peak hold.
- **Stage bar:**
  - the workflow in the edit window's title bar, for example Compose › Score › Mix › Master;
  - the current stage lit, finished stages with a green LED;
  - clicking a stage opens its tools.
- **Mixer deck:**
  - one strip per track (insert LEDs, pan, 120 px fader, stereo meter, dB readout, S/M, colored
    name plate);
  - a wider master strip outlined in the accent;
  - scrolls sideways.
- **One store drives every panel:** a change anywhere updates selection, inspector and counters
  together.
- **Narrow windows:** under about 1280 px the top-deck panels wrap, and the inspector moves.
- **Fonts:** uppercase condensed labels (Barlow Condensed), a mono face for numbers, Inter for UI
  text.

VC Composer's look is a literal hardware console: anodized gradients, screws, raised keycaps with
a 2 px drop, an amber LED color and a retro LCD face. **VC Sound Design keeps the structure but
modernizes the look:** glass panels, hairlines, flat 8 px keys, a blue accent and JetBrains Mono
readouts (see the spec's comparison table).

## Appendix C — Engine functions

Every function the app exposes, grouped by area. The spec's workflow and region tables say which
button calls which.

| Area | Functions |
|---|---|
| history | `undo`, `redo` |
| aaf | `aaf.import` |
| ai | `ai.accept`, `ai.reject`, `ai.run` |
| app | `app.info` |
| asset | `asset.peaks` |
| audition | `audition`, `audition.stop` |
| automation | `automation.clear`, `automation.delete`, `automation.mode`, `automation.set`, `automation.touch`, `automation.update` |
| bus | `bus.update` |
| clip | `clip.delete`, `clip.split`, `clip.update` |
| cue | `cue.delete`, `cue.replace`, `cue.revert`, `cue.update` |
| cues | `cues.build` |
| devices | `devices.list`, `devices.status`, `devices.test_tone` |
| export | `export.run` |
| folder | `folder.flag`, `folder.move`, `folder.rename`, `folder.ungroup` |
| fs | `fs.list` |
| identity | `identity.add`, `identity.assign` |
| input | `input.mode` |
| insert | `insert.add`, `insert.bypass`, `insert.move`, `insert.remove` |
| layout | `layout.get`, `layout.set` |
| library | `library.import`, `library.install_starter`, `library.search` |
| log | `log.get` |
| marker | `marker.add`, `marker.color`, `marker.delete`, `marker.update` |
| midi | `midi.connect`, `midi.disconnect`, `midi.message` |
| otio | `otio.import` |
| populate | `populate` (whole project, scene, category per scene, or identity everywhere) |
| perform | `perform.cancel`, `perform.commit`, `perform.select`, `perform.start`, `perform.trigger` |
| picture | `picture.import`, `picture.proxy` |
| plugin | `plugin.param`, `plugin.params` |
| plugins | `plugins.folders`, `plugins.list`, `plugins.scan` |
| prefs | `prefs.set` |
| project | `project.close`, `project.demo`, `project.new`, `project.open`, `project.recover`, `project.save`, `project.settings` |
| qa | `qa.run` |
| record | `record.start`, `record.stop` |
| relink | `relink.run` |
| report | `report.active_at`, `report.summary`, `report.write` |
| scene | `scene.add`, `scene.update` |
| score | `score.import` |
| script | `script.import` |
| spot | `spot.bind`, `spot.bindings`, `spot.finish`, `spot.tap` |
| state | `state.get` |
| take | `take.add` |
| tc | `tc.parse` |
| track | `track.update` |
| transport | `transport.locate`, `transport.loop`, `transport.play`, `transport.state`, `transport.step`, `transport.stop`, `transport.toggle` |
