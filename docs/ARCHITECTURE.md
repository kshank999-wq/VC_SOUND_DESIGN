# Architecture

## Layers

```text
 ui/ (React + TypeScript)  ──  Tauri IPC `api(cmd,args)`  ─┐
                            ──  HTTP /api/<cmd> (browser) ─┤
                                                           ▼
 crates/vcsd-engine   Engine::call — JSON command API (~80 commands)
                      player.rs   render thread ─rtrb─▶ cpal device callback (or clocked null device)
                      rack.rs     plugin instances for insert slots (synced from the project on every edit)
                      recorder.rs input capture → BWF take at the heard position
                      MIDI (midir) → live spotting / pad performance
                      devices.rs  device/port enumeration, per-user prefs (output, input, MIDI, UI layout)
                      serve.rs    browser mode + token-protected loopback media server (Range)
                                                           ▼
 crates/vcsd-plugins  VST3 host (all platforms), Audio Unit host (macOS), out-of-process scanner
                                                           ▼
 crates/vcsd-core     model, timecode, mixer, editing, reports, export … (UI-agnostic)
```

- **The audio clock is the master.** The picture follows it. `<video>` is driven by a
  proportional clock-follower: drift is corrected by nudging `playbackRate`, and only large jumps
  re-seek. When parked, the video sits mid-frame. Measured in Chromium: parked frames are exact,
  and in playback the median offset is about −0.6 frame, which is within the measurement method's
  ±0.5 frame resolution.
- **Realtime safety.** The device callback never locks the project or allocates. Blocks carry a
  seek epoch and per-frame timeline positions, so stale audio is discarded on locate and the heard
  position (minus output latency) is exact. The render thread renders at project rate and
  resamples to the device rate.
- **Automation writing.** In Touch/Latch/Write, control moves go to the engine, which stamps
  them with the heard position, monitors them live through mixer overrides (synced to the render
  thread only when they change), and on release/stop commits the pass as one undoable edit:
  replace the span, thin with Douglas–Peucker (0.25 dB / 0.01 pan), hold idle controls as steps,
  and ramp back to the existing automation over 50 ms. Loop wraps split a pass into segments.
- **Plugin hosting.** Insert slots hold VST3 plugins (macOS, Windows, Linux) and Audio Units
  (macOS). The project stores each slot's plugin id, its state (captured on save and export) and
  any parameter values set from the app, so undo works and a project opens on another computer
  with its settings. A plugin that isn't installed there keeps its state, passes audio through and
  is reported by QA. Scanning runs each new or changed bundle in a child process
  (`<app> --vcsd-plugin-scan <bundle>`), so a plugin that crashes or hangs while loading is listed
  as failed. Results are cached by modification time. Instances run on the render thread, never in
  the device callback, and exports use separate offline instances. The core mixer only sees a
  `PluginHost` trait.
- **Plugin editor windows.** In the desktop app a plugin's own editor opens in a native window
  without a webview (`crates/vcsd-app/src/editors.rs`). VST3 views are attached to the
  window's HWND, NSView or X11 window, with an `IPlugFrame` for resize requests. On Linux,
  `IRunLoop` puts the plugin's file descriptors and timers on the GTK main loop. Audio Units
  show their Cocoa view, or Apple's generic view. Window and plugin sizes are kept in step
  both ways, and closing the window or removing the slot detaches the view. Edits made in a
  plugin's window reach the processor straight away. The engine captures them into the project
  at most twice a second, so they're saved and autosaved. Before saving state, waiting
  parameter changes are flushed with a zero-sample `process` call. On Linux the app runs on
  X11 (XWayland under Wayland), because VST3 editors embed X11 windows.
- **Plugin parameter automation.** A plugin parameter's lane is owned by the plugin slot, with
  parameter `plugin:<id>` and normalised 0–1 values. It is drawn under the slot's track or bus
  and follows that owner's Read/Touch/Latch/Write mode. For each block the mixer gives the
  plugin host each automated parameter's curve: the value at the block start, every breakpoint
  inside it, and the value at its end. VST3 plugins get it as sample-accurate parameter queues,
  split along with the block. Audio Units have the parameter set every 64 samples. Writing
  uses the same passes as faders: the parameter panel's sliders send touches, and gestures in
  a plugin's own window (begin/perform/end edit, with timestamps) are placed at the timeline
  position they happened at. While a lane plays, the plugin's window and the parameter panel
  show its values. Removing a plugin removes its lanes, and undo brings both back.
- **Plugin delay compensation by reading ahead.** The timeline can be read at any position, so a
  chain reporting `L` samples of latency is fed its input `L` samples early instead of delaying
  everything else. When a chain doesn't continue from where it left off (locate, loop, a second
  export pass), it is pre-rolled with the `L` samples before so the first output isn't silence.
  Tested sample-exact against a real VST3 on track and master buses.
- **Spotting and performance timestamps come from the engine,** not from the UI: a keypress or
  MIDI note is stamped with the heard transport position.
- **Media in the webview.** Desktop builds stream picture from a loopback HTTP server with a random
  token and Range support, because WebKitGTK cannot stream video from custom URI schemes. Only
  media referenced by the open project is served.

## AAF export (Tier 2)

`crates/vcsd-core/src/aaf.rs` writes AAF directly: structured storage (4 KiB sectors) through the
`cfb` crate, and the AAF object encoding (property streams, strong-reference vectors and sets,
weak references to dictionary definitions) implemented to match pyaaf2's output byte for byte.
The fixed parts, the MetaDictionary and the Dictionary, come from `assets/aaf_template.aaf`, which
`scripts/aaf/make_template.py` generates with pyaaf2. The writer copies them and writes its own
content.

| Carried | How |
|---|---|
| Timeline position, source offset, length of every audible clip | `SourceClip` into linked media (master → file → import mobs, AMA-style) |
| Stereo tracks | Two mono tracks (L/R); stereo media is linked per channel |
| Clip gain (+ static track fader, folded in) | Mono Audio Gain `OperationGroup`, constant Amplitude |
| Linear fades in/out | `SourceClip` FadeIn/FadeOut length and type |
| Track volume automation | Mono Audio Gain around the track with a `VaryingValue` Amplitude: linear control points at exact rational times, split until amplitude-linear interpolation matches the app's dB ramps within 0.1 dB |
| Overlapping clips | Extra tracks ("Name (2)") |
| Looped clips | Butt-spliced repeats of the loop span |
| Cue markers | `CommentMarker`s ("code description") on an event track |
| Start timecode, frame rate, drop frame | Timecode track |

**Not carried** (listed in `AAF/aaf_report.json`, the README and delivery QA; all of it is in the
rendered audio): track pan automation, bus and master automation, plugin parameter
automation, pan, inserts and plugins, bus routing and bus gain, loop crossfades, muted tracks and clips (left out), and the right channel of stereo
sources on mono tracks. Media is copied into `AAF/Media/` (converted to the project rate when it
differs) and linked by absolute `file://` URL.

**Verification:** `crates/vcsd-core/tests/aaf.rs` reads every exported file back with pyaaf2
(`scripts/aaf/inspect.py`). It walks every object and checks clips, sources, fades, gain, media
links, layers, trimming, timecode and markers against the project. It also checks that the
volume curve matches the project's lane within 0.12 dB at 400 points. It has not been opened in
DaVinci Resolve, Pro Tools or Media Composer yet. Pinning a Resolve version and recording its
feature matrix is the next step.

## Principles carried into code

| Spec principle | Implementation |
|---|---|
| Picture/timecode is truth; integer samples | `timecode.rs`: rational `FrameRate {num, den, drop_frame}`, exact i128 frame↔sample mapping (frame *f* = samples `[ceil(f·k), ceil((f+1)·k))`), DF/NDF labels, `TimeBase` with non-zero start TC. Round-trip tested at 23.976/29.97 DF/59.94 DF at 44.1/48/96 kHz with no drift over 10 h. |
| Non-destructive, reversible | Every mutation runs through `Session::edit` → snapshot, atomic rollback on error, undo/redo, audit log. Media files are referenced and never written. |
| Stable IDs | UUIDv7 with entity prefixes (`cue_…`, `clp_…`), human cue codes (`12-FST-003`), deterministic sort (start sample → code → id). |
| Reusable identities | `SoundIdentity` with default asset, variation pools (round-robin / seeded random / locked / alternate feet), scene-local overrides on the cue, and confirmed ripple across the film. |
| Never trapped by placeholders | Clip `history` keeps every asset a clip has used. `revert_to_placeholder` keeps takes, and repopulate never touches custom clips. |
| AI suggests, never commits | `ai.rs`: `TimingAssistant` trait. Proposals are stored separately and cached per picture fingerprint; nothing changes until accepted. Off by default; cloud needs explicit consent. |
| Portable output | SQLite project, BWF WAV, JSON/CSV manifests, landscape PDF. No proprietary formats. |

## Modules (`crates/vcsd-core/src`)

| Module | Spec | Responsibility |
|---|---|---|
| `timecode` | §13 | Frame rates, DF/NDF, sample clock |
| `model`, `state` | §8.1, §17 | Entities, `ProjectState`, `Session` (undo/redo/autosave/recovery) |
| `store`, `layout` | §15.3, §16 | `Project.vcsd` SQLite (one table per entity, schema version, migration hook, atomic temp-file+rename save), folder layout |
| `wav`, `hash` | §11.2, §15 | WAV/BWF read/write (16/24/32/float, `bext` time reference), SHA-256, picture fingerprints |
| `picture` | §5.2, §13 | Import with ffprobe TC/rate probing, versions, reconform as review flags with explicit apply |
| `library`, `synth` | §9 | WAV indexing, packs (`pack.json`), search, variation picks, procedural starter pack (owned, redistributable) |
| `spotting` | §6.2, §8 | Scenes, marker lanes, point/range/repeated markers, `LiveSpotter` (hotkeys + MIDI note-on, snap-to-frame), markers → cues, identities |
| `builder` | §8.3 | Auto-track builder with track policies, labels (`FOLEY • Greg • Boots`), loop/one-shot/inferred-step fill, fit warnings, preview mode |
| `editing` | §8.3, §11.2, §12 | Replace (cue / identity in scene / ripple), revert, move/trim/slip/split/fades/crossfade/gain, automation (incl. write passes), track folders (path-based: move, rename, ungroup, folder mute/solo), takes (BWF placement, sync offset) |
| `perform` | §11.1 | Pad maps (key/MIDI/touch), trigger capture with latency compensation, optional quantize, velocity, deterministic re-render, take selection |
| `mixer` | §12 | Default department buses; render engine (clip gain, fades, crossfaded loops, channel mapping, constant-power pan, volume/pan automation on tracks, stem buses and master (breakpoints, linear in dB, amortised O(1) lookup per sample), HPF/LPF/peak EQ and compressor inserts, bus tree), freeze |
| `transport` | §12, WP13 | Sample-clock transport, sample-accurate loop wrap, frame step, latency figures, `AudioDevice` trait, block playback that matches offline render exactly |
| `ai` | §10 | Cadence heuristic and ffmpeg scene-score visual-onset assistant; low-confidence reporting; accept/adjust/reject |
| `integrations` | §7, §12 | VC Film Studio breakdown import/re-import (conflicts surfaced) and sound-field round-trip; VC Composer score import with TC/rate/BWF/scene mismatch detection |
| `report`, `pdf` | §14 | Cue rows, CSV/JSON (FR + DF/NDF embedded), active-at-playhead, scene summary, dependency-free landscape PDF (overview lanes, paginated table, page numbers, missing-media warnings) |
| `qa` | §15.2 | Missing media, hash verification, overlaps, out-of-range, channels, SRC, rights, temp references, score sync (blocking), reconform review, pending AI |
| `export`, `handoff` | §15 | Quick / Resolve / Pro / Archival profiles, BWF-stamped aligned tracks/stems/mix, discrete cues with handles, manifests, README, checksums with verification; `InterchangeAdapter` trait for Tier 2; package → timeline reconstruction |

## Acceptance coverage (spec §20)

| # | Test |
|---|---|
| 22 | `s22_one_frame_event_survives_save_reopen` |
| 23 | `s23_identity_across_scenes_with_isolated_override` |
| 24 | `s24_eight_midi_taps_become_editable_events` (both live spotting and the performance recorder) |
| 25 | `s25_replace_instance_with_recorded_take_keeps_history` |
| 26 | `s26_rejected_ai_proposals_change_nothing` |
| 27 | `s27_score_timecode_mismatch_blocks_export` |
| 28 | `s28_overlap_report_and_playhead_query` |
| 29, 31 | `s29_s31_export_reimport_alignment_and_reconstruction` (≤ ½ LSB at 24-bit, zero lag) |
| 30 | `s30_relink_moved_library_by_hash` |

## Decisions taken (spec §22)

- **Engine language and shell:** a Rust engine (deterministic and realtime-capable, cross-platform) inside a Tauri 2 desktop shell with a React UI. The webview only draws the interface; audio, recording and MIDI run natively in the engine, which follows the spec's rule against making browser JavaScript responsible for realtime audio.
- **Native format:** a versioned SQLite DB with JSON documents per entity and external media, as the spec proposes.
- **Placeholder licensing:** the bundled starter pack is generated procedurally by our own code, so it can be redistributed. Third-party packs carry their own license metadata, and QA flags anything not redistributable.
- **Interchange:** Tier 1 (aligned BWF plus manifest) is implemented and tested. Tier 2 is an optional AAF of the editable timeline (see below), checked with an independent reader but not yet against a pinned Resolve/Fairlight version.

## Known limitations

- Undo history is in memory only and is not persisted across sessions. Snapshots copy the whole project state, which is fine at MVP scale.
- Sample-rate conversion is linear, and QA flags it as not bit-accurate.
- WAV files over 4 GiB (RF64) are not written.
- Reconform needs an explicit change list. EDL/XML change-list parsing is not done yet.
- Script import is the JSON contract only. The PDF/text fallback is not done yet.
- XLSX export (optional in the spec) is not done.
- Plugin editor windows are tested end to end on Linux (X11) only. On macOS and Windows the
  code is type-checked, and the Audio Unit editor has not run yet. Browser mode (`vcsd serve`)
  has the parameter panel only.
- Changes made in a plugin's own window outside an automation pass are not undoable (the
  plugin's state is saved, not each move).
- Audio Unit parameter automation is applied every 64 samples; VST3 is sample-accurate.
- Plugin lanes show values as 0–100 %, not in the plugin's own units.
- Plugin parameters can't be automated yet (track/bus volume and pan can).
- Inserts are effects only: instruments, sidechain inputs and MIDI to plugins are not routed.

## Roadmap

1. **Plugins, next steps:** a test pass with common commercial plugins (editors and
   automation) on each OS, and sample-accurate Audio Unit automation
   (`AudioUnitScheduleParameters`).
2. **Hardware validation:** test the device matrix (ASIO on Windows via a cpal feature, aggregate devices on macOS) and measure round-trip latency for recording compensation. Add punch-in/out and multi-input recording.
3. **Distribution:** cloud-HSM Windows signing (`signCommand`), a beta update channel, and delta updates.
4. **Performance:** streaming disk reads instead of full decode, a waveform/peak cache, and CPU/RAM/disk/dropout meters (`transport::perf_probe` is the starting point).
5. **Interchange:** open the AAF export in a pinned Resolve/Fairlight version (and Pro Tools), record the feature matrix (volume automation especially: hosts differ in whether they read
track-level gain operations), then OTIO.
6. **AI:** optical-flow or pose models for footfall detection behind the existing trait, plus an opt-in cloud provider with consent and retention controls.
