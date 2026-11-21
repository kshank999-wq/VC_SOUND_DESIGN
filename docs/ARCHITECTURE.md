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
| Stereo tracks | Two mono tracks hard-panned L/R; stereo media is linked per channel |
| Pan on mono tracks, static or automated | Mono Audio Pan (Pan = (pan + 1) / 2) around the track; the breakpoints map exactly because both sides interpolate pan linearly. The receiving app's pan law applies (the app uses −3 dB constant power) |
| Balance on stereo tracks, static or automated | Folded into each side's gain curve (with any volume automation), fitted within 0.1 dB, including the corner where the balance crosses centre |
| Bus and master levels: static gain, volume automation, balance automation | AAF has no buses, so each bus's levels down to the master are multiplied into the gain of every track it carries. On mono tracks a bus balance moves the panned signal's left/right; with a −3 dB constant-power pan law that is exactly a new pan plus a gain, which is written (pan fitted within 0.002, and each side within 0.1 dB wherever it is above −30 dB). Tracks on muted buses are left out |
| Clip gain (+ static track fader, folded in) | Mono Audio Gain `OperationGroup`, constant Amplitude |
| Linear fades in/out | `SourceClip` FadeIn/FadeOut length and type |
| Track volume automation | Mono Audio Gain around the track with a `VaryingValue` Amplitude: linear control points at exact rational times, split until amplitude-linear interpolation matches the app's dB ramps within 0.1 dB |
| Overlapping clips | Extra tracks ("Name (2)") |
| Looped clips | Butt-spliced repeats of the loop span |
| Cue markers | `CommentMarker`s ("code description") on an event track |
| Plugins, insert effects and plugin parameter automation (option "print tracks with plugins") | AAF can't carry VST3/AU plugins, so tracks with active inserts are rendered pre-fader (clips → inserts with their automation and delay compensation) to `AAF/Media/*_print.wav` and linked as one clip each. Their fader, volume, pan and bus levels stay editable around the print; their clips don't |
| Start timecode, frame rate, drop frame | Timecode track |

**Not carried** (listed in `AAF/aaf_report.json`, the README and delivery QA; all of it is in the
rendered audio): plugins and insert effects on buses, and on tracks unless printed, with
their automation, bus routing and bus gain, loop crossfades, muted tracks and clips (left out), and the right channel of stereo
sources on mono tracks. Media is copied into `AAF/Media/` (converted to the project rate when it
differs) and linked by absolute `file://` URL.

**Verification:** `crates/vcsd-core/tests/aaf.rs` reads every exported file back with pyaaf2
(`scripts/aaf/inspect.py`). It walks every object and checks clips, sources, fades, gain, media
links, layers, trimming, timecode and markers against the project. It also checks that the
volume curve matches the project's lane within 0.12 dB at 400 points. Mono pan is checked as
exact, and stereo balance gains within 0.12 dB across a move that crosses the centre. With bus
gain, bus balance and master automation, the left/right levels a constant-power host plays from
the AAF match the mixer's within 0.15 dB. A printed track is checked sample by sample against
the dry clips through the plugin and its automation. It has not been opened in
DaVinci Resolve, Pro Tools or Media Composer yet. Pinning a Resolve version and recording its
feature matrix is the next step.

## OTIO export (Tier 2)

`crates/vcsd-core/src/otio.rs` writes the editable timeline as OpenTimelineIO JSON
(`OTIO/<name>.otio`; schemas `Timeline.1`, `Stack.1`, `Track.1`, `Clip.2`, `Gap.1`,
`Transition.1`, `ExternalReference.1`, `Marker.2`, readable by OpenTimelineIO 0.15 and later).
It shares the AAF's layout rules (track order, overlapping clips on extra tracks, loops as
butt-spliced repeats, muted tracks and clips left out). It also shares its media: copies in
`AAF/Media/`, or `OTIO/Media/` when there is no AAF, and the prints of tracks with plugins.

| Project | OTIO |
|---|---|
| Track | An audio track (OTIO has no channel count: a stereo track is one track of stereo clips; `channels` in its metadata) |
| Clip | A clip with its exact source range, linking its WAV (`ExternalReference`, absolute `file://` URL, available range = the file). Times are in samples, rate = the sample rate |
| Start timecode | `global_start_time` at the project frame rate |
| Fade next to silence | An `SMPTE_Dissolve` transition against the gap or track edge (in/out offset = the fade) |
| Fade between butted clips | Clip metadata only (a transition there would be a crossfade over handles) |
| Clip gain | A "Mono Audio Gain" `Effect` on the clip, with a constant `Amplitude` (linear, as the AAF Rational string `"n/d"`) |
| Track fader × volume automation × every bus level down to the master (static gain and volume automation) | A "Mono Audio Gain" `Effect` on the track: a constant, or keyframes `[fraction of the track's length, linear gain]`, linear in amplitude and fitted within 0.1 dB of the mixer (whose lanes ramp in dB) |
| Track pan, pan automation, and the buses' balance automation | A "Mono Audio Pan" `Effect` on the track (`Pan`: 0 left, 0.5 centre, 1 right), constant or keyframed. Without bus balance, the lane's breakpoints are the keyframes, exactly (it is linear in its value, as AAF keyframes are). A bus balance on a mono track is a new pan plus a gain, exact with a −3 dB constant-power pan law, as in the AAF; it is fitted so each side stays within 0.1 dB above −30 dB. On a stereo track the value is the mixer's balance (`metadata.vcsd.law`): the louder side's level is the gain, the other side's ratio the balance, exactly under the balance law |
| Clip fades, bus, and the raw fader, pan and lanes (offsets from the range start) | `metadata.vcsd` on the clip or track |
| Cues | Timeline markers, coloured by category, with the description as comment |

OTIO has no audio mix model, so levels follow the one convention OTIO tools share. The
OpenTimelineIO AAF adapter carries AAF's Mono Audio Gain as an `Effect` whose
`metadata.AAF.Parameters.Amplitude` is a constant or keyframes (with `simplify=True` it sits on the
track or clip); Mono Audio Pan is the same, with `Pan`. The export writes levels and pan in that
encoding. OTIO has no buses either, so, as in the AAF, each track's effects include what its buses
do to it. `metadata.vcsd.bus_folded` says when they do, and the import then takes the track's
levels from the effects rather than its raw fader and lanes. Bus routing and inserts are not
carried. Pan sounds as the receiving app's pan law makes it. The export lists these in
`OTIO/otio_report.json`, the README and QA.

**Verification:** `crates/vcsd-core/tests/otio.rs` reads each file with the OpenTimelineIO
library (`scripts/otio/otio_summary.py`) and checks the following:
- the start timecode, and that every track spans the range;
- every audible clip's position, length and source offset, and that its media exists;
- the overlapping layer and the left-out muted track;
- the fade dissolves around the glass clip;
- the clip's gain effect, and the track's gain keyframes against the mixer's level (fader plus
  lane) within 0.12 dB at 401 points;
- the pan keyframes against the pan lane, exact at 401 points, and a stereo track's balance as a
  constant pan;
- with bus gain, bus volume automation and master balance automation crossing the centre, the
  left and right levels a host plays from the effects (constant-power pan on the mono track,
  balance on the stereo one) against the mixer's, within 0.15 dB at 401 points. A side more than
  30 dB under the other is checked within 0.1 % of the louder side instead;
- the markers;
- sharing the media and prints with the AAF.

It has not been opened in an editor yet.

## OTIO import

`crates/vcsd-core/src/otio_import.rs` reads `.otio` JSON: a `Timeline` (or the first one in a
`SerializableCollection`), with `Clip.1` or `Clip.2` media references. It rebuilds the audio
tracks as new tracks in a folder `OTIO/<timeline>`, placed at `global_start_time` or at a given TC:

| OTIO | Project |
|---|---|
| Audio track (video tracks are ignored) | A track, its category guessed from the name; stereo when its media is |
| Clip, at any rate | A clip with its source range, converted to samples. When the media's available range starts at a timecode (BWF media from editors), the offset into the file is relative to that |
| Transition next to a gap | A fade in or out of `in + out` |
| Transition between clips | A crossfade: the clip before runs on past the cut by the out offset, the clip after starts before it by the in offset (into their handles, as OTIO defines it). Handles the media doesn't have shorten it |
| Nested stack (compound clip) | Its audio tracks as extra tracks ("outer / inner"), trimmed to the stack's range |
| Disabled track or clip | Muted |
| Markers on the timeline, tracks and clips | Spotting markers in an "OTIO markers" lane (name — comment), for those inside a scene |
| `metadata.vcsd` (our own export) | Clip gain and fades, track fader, pan, channels, volume and pan automation |
| Without it: AAF-style "Mono Audio Gain" / "Mono Audio Pan" effects on tracks and clips (the AAF adapter's encoding, and our export's) | Constant clip gain becomes clip gain. Track gain and pan, and keyframed clip gain, become the track's gain, pan and volume/pan lanes (gains multiply, fitted within 0.1 dB) |

**Media:** `file://` URLs, absolute paths, or paths relative to the `.otio`. WAV/BWF is used
where it is; moved media is searched for by name next to the file and in the given folders.
AIFF and MXF are converted like the AAF import's, as is media at another sample rate. Files are
written to `Audio/Imported/OTIO/<file>/`. Missing references and unreadable media are reported
per file.

Files with neither our metadata nor gain effects come in at unity gain, centred, and the report says
so.

**Verification:** `crates/vcsd-core/tests/otio_import.rs` round-trips an OTIO export, with no
library needed. The imported mix matches the original to within 1e-5 per sample, and the
automation lane comes back exactly. It also imports a timeline written with the OpenTimelineIO
library in an editor's layout (`scripts/otio/make_fixture.py`) and checks the following:
- 25 fps, media ranges at 10:00:00:00 and a relative URL;
- a fade in from a gap, a dissolve with handles, and a disabled clip;
- a moved AIFF found by name and byte-swapped correctly;
- a nested stack trimmed to its range;
- a missing reference reported, and the timeline and clip markers.

Levels and pan are checked two ways, each at 201 points of the automated clip (levels within
0.25 dB, pan within 0.003):
- our export with its `metadata.vcsd` stripped, read from the gain effects alone;
- our AAF converted to OTIO by the OpenTimelineIO AAF adapter, then imported. The fader folded
  into clip gain comes back as −9 dB.

With bus levels and master balance folded in, importing into a project with flat buses gives the
same master mix within 2 % of peak.

## AAF import

`crates/vcsd-core/src/aaf_import.rs` reads AAF files from editors' systems: little-endian
structured storage, as written by the AAF SDK (Media Composer, Pro Tools, Premiere, Resolve),
pyaaf2 and VC Sound Design. It reads the object tree generically: property streams,
strong-reference vectors and sets, weak-reference keys and data streams. It recognises classes
by their storage CLSID, walking up the file's meta-dictionary for extension subclasses. The
top-level composition (by name, or the one with the most sound clips) is placed at its timecode
(or a given TC) and becomes new tracks in a folder `AAF/<composition>`:

| AAF | Project |
|---|---|
| Sound track (any edit rate: positions are converted exactly, e.g. 1 frame at 25 fps = 1920 samples) | A track, its category guessed from the name ("DIA 1" → Dialogue). Picture tracks are ignored |
| Two tracks playing channels 1 and 2 of the same stereo files, hard-panned L/R (or named "… L"/"… R") | One stereo track; the two sides' gains become volume plus balance |
| `SourceClip` (through selectors, essence groups, nested scopes and effects) | A clip. Its source is followed through master and file mobs (edit rates and slot origins on the way) to a file and channel |
| Clip fades; `Transition` | Fades; a transition is a crossfade of its length |
| Mono Audio Gain on a clip (constant) | Clip gain |
| Mono Audio Gain around a track (constant or varying), or a varying gain on a clip | Track gain, or a volume lane. AAF interpolates linear amplitude, so the lane is fitted within 0.1 dB, then thinned (collinear breakpoints dropped) |
| Mono Audio Pan (constant or varying) | Track pan or a pan lane |
| Comment and descriptive markers | Spotting markers in an "AAF markers" lane, for those inside a scene |

**Media:** linked WAV/BWF is used where it is. When it has moved, it is searched for by name next
to the AAF and in the given folders. Embedded essence is extracted to
`Audio/Imported/AAF/<file>/`, bit-exact: WAVE, AIFC, or raw PCM per the descriptor. So are
AIFF/AIFC and MXF sound essence, and a channel of a multichannel file used on a
mono track. Media at another sample rate is converted (linear). Files are named after their mob,
so importing again reuses them, and assets are deduplicated by content.

MXF sound essence comes in three forms:
- BWF elements (SMPTE 382, frame- or clip-wrapped), as in Avid OP-Atom and most files.
- SMPTE 382 AES3 elements. Their samples are stored as in BWF; the AES3 channel status is
  descriptor metadata.
- D-10/IMX AES3 elements (SMPTE 331/386). Each element is a 4-byte header, then eight channel
  slots per sample of 32-bit AES3 subframes. The audio sits in bits 4–27 and is kept at 24 bits,
  or at its top 16 bits for 16-bit audio. The channel and V/U/C/P bits are dropped.

A file can carry several sound tracks: OP1a has one element per track in each content package,
or one clip-wrapped element each. Elements are told apart by their track number (the last four
bytes of the key). Each track is decoded with its own descriptor, found by following track
number → timeline track ID → the descriptor's `LinkedTrackID`. When nothing links them, a single
descriptor (or identical ones) serves every track. The tracks become one WAV, their channels in
track order, so the AAF file mob's channel `k` (its PhysicalTrackNumber) is the file's channel
`k`, and it is extracted like any multichannel file's. Tracks of different bit depths are
written at the widest one, exactly. A shorter track is padded with silence. Tracks at different
sample rates, or with different parameters and nothing linking them, are reported as unreadable.
Picture and other elements in the same file are skipped. Sample rate, channel count and bit
depth come from the MXF's sound descriptor (generic, AES3 or wave), else from the AAF's. For
D-10 without either, the channel count comes from the element's valid flags.

**Reported, not imported:** media that can't be found or read (per file, with the clip count),
effects other than gain and pan, non-linear interpolation (read as linear), and markers outside
every scene.

**Verification:** `crates/vcsd-core/tests/aaf_import.rs` round-trips an export: the master mix
of the imported project matches the original within the curve fits. It also imports an AAF
written by pyaaf2 in an editor's layout (`scripts/aaf/make_fixture.py`) and checks the
following:
- 25 fps edit units, the transition as a crossfade, and a bit-exact channel extract of
  embedded PCM;
- moved linked media found by name or in a search folder;
- channel 3 of a linked two-track OP1a MXF (mono, then stereo), bit-exact;
- clip gain, the varying track gain against the AAF curve, pan, and the marker.

It has not yet been tested with files from Media Composer, Pro Tools or Resolve.

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
- **Interchange:** Tier 1 (aligned BWF plus manifest) is implemented and tested. Tier 2 is an optional AAF of the editable timeline (see below), checked with an independent reader but not yet against a pinned Resolve/Fairlight version, and optionally the same timeline as OTIO (checked with the OpenTimelineIO library). AAF and OTIO import reads editors' timelines (tested with pyaaf2-written files and round trips, not yet with files from Media Composer, Pro Tools or Resolve).

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
5. **Interchange:** open the AAF export in a pinned Resolve/Fairlight version (and Pro Tools), import AAFs exported by Media Composer, Premiere, Resolve and Pro Tools (with real AES3/D-10 and multitrack OP1a MXF), record the feature matrix (volume and pan automation especially: hosts differ in whether they
read track-level gain/pan operations, and in their pan laws), and the OTIO export in Resolve and an OTIO-based conform tool.
6. **AI:** optical-flow or pose models for footfall detection behind the existing trait, plus an opt-in cloud provider with consent and retention controls.
