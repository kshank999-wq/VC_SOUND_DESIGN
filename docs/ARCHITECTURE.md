# Architecture

## Layers

```text
 ui/ (React + TypeScript)  ──  Tauri IPC `api(cmd,args)`  ─┐
                            ──  HTTP /api/<cmd> (browser) ─┤
                                                           ▼
 crates/vcsd-engine   Engine::call — JSON command API (~70 commands)
                      player.rs   render thread ─rtrb─▶ cpal device callback (or clocked null device)
                      recorder.rs input capture → BWF take at the heard position
                      MIDI (midir) → live spotting / pad performance
                      devices.rs  device/port enumeration, per-user prefs (output, input, MIDI)
                      serve.rs    browser mode + token-protected loopback media server (Range)
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
- **Spotting and performance timestamps come from the engine,** not from the UI: a keypress or
  MIDI note is stamped with the heard transport position.
- **Media in the webview.** Desktop builds stream picture from a loopback HTTP server with a random
  token and Range support, because WebKitGTK cannot stream video from custom URI schemes. Only
  media referenced by the open project is served.

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
| `editing` | §8.3, §11.2, §12 | Replace (cue / identity in scene / ripple), revert, move/trim/slip/split/fades/crossfade/gain, automation, takes (BWF placement, sync offset) |
| `perform` | §11.1 | Pad maps (key/MIDI/touch), trigger capture with latency compensation, optional quantize, velocity, deterministic re-render, take selection |
| `mixer` | §12 | Default department buses; render engine (clip gain, fades, crossfaded loops, channel mapping, constant-power pan, volume/pan automation, HPF/LPF/peak EQ and compressor inserts, bus tree), freeze |
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
- **Interchange:** Tier 1 (aligned BWF plus manifest) is implemented and tested. Tier 2 is an adapter trait only, pending a pinned-version Fairlight spike.

## Known limitations

- Undo history is in memory only and is not persisted across sessions. Snapshots copy the whole project state, which is fine at MVP scale.
- Sample-rate conversion is linear, and QA flags it as not bit-accurate.
- WAV files over 4 GiB (RF64) are not written.
- Reconform needs an explicit change list. EDL/XML change-list parsing is not done yet.
- Script import is the JSON contract only. The PDF/text fallback is not done yet.
- XLSX export (optional in the spec) is not done.

## Roadmap

1. **Hardware validation:** test the device matrix (ASIO on Windows via a cpal feature, aggregate devices on macOS) and measure round-trip latency for recording compensation. Add punch-in/out and multi-input recording.
2. **App polish:** clip trim/fade handles on the timeline, automation lanes, track folders, persisted window layout, signed installers and auto-update.
3. **Performance:** streaming disk reads instead of full decode, a waveform/peak cache, and CPU/RAM/disk/dropout meters (`transport::perf_probe` is the starting point).
4. **Interchange spike:** AAF/OTIO against a pinned Resolve version, implemented as an `InterchangeAdapter` that reports what it loses.
5. **AI:** optical-flow or pose models for footfall detection behind the existing trait, plus an opt-in cloud provider with consent and retention controls.
