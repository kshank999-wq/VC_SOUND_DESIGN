# VC Sound Design

A sound-design-first DAW engine for picture: spot sounds against an edited scene, build a
cue chart, populate placeholder audio, perform or record Foley, rough-mix, and hand off
sample-aligned stems with a machine-readable cue manifest for DaVinci Resolve/Fairlight or a
finishing studio.

The full requirements are in [`docs/spec/`](docs/spec/VC_Sound_Design_Development_Spec_v1.0.md).
How the code maps onto them is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Status

- **Desktop app** (`crates/vcsd-app`, Tauri 2 + React): picture viewer slaved to the audio
  clock, layered marker lanes and tracks with waveforms, live keyboard/MIDI spotting, cue chart,
  inspector, library browser with audition, perform-to-picture pads, recording, inline faders,
  a detachable mixer window, QA and export.
- **Engine** (`crates/vcsd-engine`): realtime playback through CoreAudio/WASAPI/ALSA (cpal),
  input recording, MIDI input, and one JSON command API used by the desktop app and browser mode.
- **Core** (`crates/vcsd-core`): spec phases P0–P3, the P4 integration contracts and the P5
  AI-proposal pipeline. Every spec §20 acceptance scenario (22–31) is an automated test.

**Not built yet:** AAF/Tier-2 interchange, plugin hosting (VST3/AU), signed installers and
auto-update. See the [roadmap](docs/ARCHITECTURE.md#roadmap).

## Desktop app

Prerequisites: Rust (stable), Node 20+, and on Linux the WebKitGTK/ALSA packages
(`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libasound2-dev`). macOS and Windows need no
extra packages. `ffmpeg` on `PATH` is optional; it enables picture timecode probing, playback
proxies and visual-onset AI assist.

```bash
npm --prefix ui ci
```

```bash
npm --prefix ui run build
```

```bash
cargo run --release -p vcsd-app
```

To open a project directly, pass its folder: `cargo run --release -p vcsd-app -- ~/Films/GravelRoad`.
To produce an installer (.dmg/.app, .msi/.exe, .deb/.AppImage), run `npx tauri build` from
`crates/vcsd-app` using the CLI installed in `ui/node_modules`.

On first launch, choose **Create demo project…** to get a fully spotted 3-scene project.

### Keyboard

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
| Drag a clip / its left or right edge | Move / trim (audio stays in place; edits snap to frames) |
| Drag the square handle at a clip's top corner | Fade in / fade out (double-click the handle to remove the fade) |
| Track header **A** | Show/hide that track's Volume and Pan automation lanes |
| Bus header **A** (Buses section, below the tracks) | Volume and Balance automation for a stem bus or the master |
| Click an automation lane / drag a breakpoint / double-click it | Add (on the line: keeps its value) / move (Shift: value only) / delete |
| Mode button **R/T/L/W** (track/bus header or mixer strip) | Automation mode: **Read**, **Touch** (write while held, return on release), **Latch** (write from first touch, hold to stop), **Write** (whole pass, play → stop) |
| Move a fader/pan in Touch/Latch/Write while playing | Records automation against the audio clock, heard live; committed as one undo step |
| Hold Alt while dragging | Disable frame snapping |
| Shift-drag in the ruler | Loop range |

### Audio & MIDI devices

Open **☰ → Audio & MIDI devices…** (or click the device name in the status bar):

- **Output:** any connected device or *System default*. Switching takes effect immediately,
  even during playback. If a saved device is unplugged, the app falls back to the default and
  shows a ⚠ in the status bar. *Test tone* plays 1 kHz at −18 dBFS.
- **Input:** the device used for Foley recording.
- **MIDI input:** the controller used for live spotting and pad performance, with an activity
  light and the last message received. The choice is remembered and reconnects at launch.

These settings belong to the computer, not the project. They are saved in `prefs.json` under
the user config folder (`~/.config/vc-sound-design`, `~/Library/Application Support/VC Sound
Design`, or `%APPDATA%\VC Sound Design`; override with `VCSD_CONFIG_DIR`).

### Browser mode

The same UI also runs in a web browser against a local engine, which is useful on a headless
machine or for testing:

```bash
cargo run --release -p vcsd-cli -- serve ./demo_project --ui ui/dist
```

Then open http://127.0.0.1:7878/. It binds to localhost only and serves only the open project's media.

## CLI quick start

```bash
cargo build --release
```

```bash
./target/release/vcsd demo ./demo_project
```

`demo` runs the spec §5 reference workflow: it generates a timecoded test picture (if ffmpeg
is installed) and a procedural starter pack. Then it spots three scenes with simulated
hotkey/MIDI taps, builds the cue chart, populates placeholders, sets a scene-local override,
performs Foley on a pad, drops in a recorded take, imports a Composer score, runs AI timing
assist, and exports the Resolve and Pro-studio packages.

## A manual session

```bash
vcsd new film --name "Gravel Road" --fps 23.976 --sr 48000 --start-tc 01:00:00:00
vcsd picture film cut_v1.mov --label v1
vcsd pack ./VC_Starter_Pack && vcsd lib film ./VC_Starter_Pack --placeholder
vcsd scene film 12 01:00:00:00 01:00:20:00 --title "EXT. GRAVEL DRIVE"
vcsd identity film "Greg / Boots" --category footsteps --pool "boot gravel" --mode alternate
vcsd spot film --lane footsteps --kind range --in 01:00:03:00 --out 01:00:09:00 --identity "Greg / Boots" --label "Greg walks"
vcsd spot film --lane fx --kind point --in 01:00:12:05 --label "glass smash"
vcsd cues film
vcsd populate film
vcsd replace film 12-FX-002 "glass smash 02"
vcsd report film scene12.pdf --scene 12
vcsd qa film --hashes
vcsd export film --profile resolve
```

Run `vcsd --help` for every command (takes, revert, relink, score/script import, AI proposals,
reconstruct from a package, timecode conversion).

## Layout

| Path | Contents |
|---|---|
| `crates/vcsd-core` | Engine library: timecode, model, SQLite project store, spotting, builder, editing, performance, mixer, AI, integrations, reports/PDF, QA, export, handoff |
| `crates/vcsd-core/tests` | `acceptance.rs` (spec §20) and `workflow.rs` |
| `crates/vcsd-engine` | App services: command API, realtime player, recording, MIDI, browser-mode server |
| `crates/vcsd-app` | Tauri desktop shell |
| `ui/` | React/TypeScript interface (shared by desktop and browser mode) |
| `crates/vcsd-cli` | `vcsd` command-line front end (`demo`, `serve`, …) |
| `docs/` | Spec and architecture notes |

## Tests

```bash
cargo test --release
```

```bash
npm --prefix ui test
```

The visual-onset AI test needs `ffmpeg` on `PATH` and skips itself when it is missing. On
Linux, `crates/vcsd-engine/tests/devices.rs` creates virtual ALSA devices, so device switching,
playback and recording run through real ALSA streams without sound hardware.
`ui/e2e/workflow.cjs` drives the UI end to end with Playwright against a running `vcsd serve`
(live spotting while playing, cue building, populate, replace + undo, pad performance, export,
detached mixer); `ui/e2e/handles.cjs`, `ui/e2e/automation.cjs`, `ui/e2e/bus_automation.cjs` and
`ui/e2e/write_automation.cjs` drag real trim/fade handles, automation breakpoints and mixer faders
(writing Latch/Touch automation during playback) with the mouse.
