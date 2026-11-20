# VC Sound Design

A sound-design-first DAW engine for picture: spot sounds against an edited scene, build a
cue chart, populate placeholder audio, perform or record Foley, rough-mix, and hand off
sample-aligned stems with a machine-readable cue manifest for DaVinci Resolve/Fairlight or a
finishing studio.

The full requirements are in [`docs/spec/`](docs/spec/VC_Sound_Design_Development_Spec_v1.0.md).
How the code maps onto them is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Status

This repository is the **core engine (spec phases P0–P3, plus the P4 integration contracts and
the P5 AI-proposal pipeline)** as a Rust library with a CLI. It is UI-agnostic, so a desktop
shell sits on top of it. Every acceptance scenario in spec §20 (22–31) is an automated test.

**Not built yet:** the desktop GUI (video viewer, timeline, detachable mixer), realtime audio
and MIDI device back-ends (only the interfaces exist), live input capture (takes are imported
WAVs), AAF/Tier-2 interchange, and plugin hosting. See
[Roadmap](docs/ARCHITECTURE.md#roadmap).

## Quick start

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

```bash
./target/release/vcsd cues ./demo_project
```

```bash
./target/release/vcsd at ./demo_project 01:00:27:14
```

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
| `crates/vcsd-cli` | `vcsd` command-line front end and `demo` |
| `docs/` | Spec and architecture notes |

## Tests

```bash
cargo test --release
```

The visual-onset AI test needs `ffmpeg` on `PATH` and skips itself when it is missing.
