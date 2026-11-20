Sound-design-first digital audio workstation • AI-assisted spotting •
Foley performance • professional handoff

Audience: coding agent, product engineering, audio engineering, UX,
integration and QA teams. Status: initial implementation baseline;
proposed technical choices are labeled as recommendations.

# 1. Product mandate

Build a standalone, picture-synchronized digital audio workstation (DAW)
purpose-built for sound designers, Foley artists, editors and
independent film teams. Its signature workflow is: import an edited
scene → spot sounds using layered markers → build a sound cue chart →
populate editable tracks with library placeholders → replace, perform or
record final Foley and effects → refine in a timeline and mixer → export
portable audio, metadata and scene reports for DaVinci Resolve/Fairlight
or a larger finishing facility.

This product is NOT intended to replace a major theatrical final-mix
stage. It should let smaller productions do organized, high-quality
sound editorial and rough mixing, preserving everything needed for a
professional downstream mix and mastering handoff.

# 2. Product principles

-   Manual-first, AI-assisted: human users choose the sound vocabulary
    and intent; AI may propose event timing, never silently commit
    irreversible edits.

-   Picture and timecode are the source of truth; soundtrack changes are
    non-destructive and reversible.

-   Sound identities are reusable across scenes: e.g., "Greg / Boots /
    Gravel" retains a stable identity, shared library assignment and
    searchable history.

-   One-click placeholder population must never trap users: every cue
    can be swapped, edited, re-timed, recorded or re-performed.

-   Track count is software-unlimited, bounded in practice by machine
    capacity; show performance indicators rather than artificial caps.

-   Native project, media, cue manifest and exported assets must remain
    portable, including when a downstream format cannot retain every
    feature.

-   All generative features must label provenance, show confidence and
    offer review. Offline/manual core operation must not depend on AI.

# 3. Scope and terminology

  -----------------------------------------------------------------------
  Term                                Definition
  ----------------------------------- -----------------------------------
  Scene                               Script/film unit with scene number,
                                      picture range and its own cues and
                                      report.

  Sound identity                      A recurring conceptual sound source
                                      (e.g., Greg's boots), reusable
                                      across the production.

  Cue                                 One intended sound event or
                                      interval with unique ID, in/out
                                      timecode, category, notes and
                                      assets.

  Spotting marker                     Point or bounded in/out region on
                                      the master picture timeline,
                                      optionally grouped in marker lanes.

  Placeholder                         Default licensed/generic audio
                                      asset automatically placed until
                                      refined or replaced.

  Instance                            A placed asset occurrence
                                      associated with a cue; many
                                      instances may reference one source
                                      asset.

  Stem                                A rendered grouped output (e.g.,
                                      Foley, backgrounds, hard FX);
                                      export definition configurable.

  Handoff package                     Audio plus machine-readable
                                      cue/track inventory, reports and
                                      optional interchange timeline.
  -----------------------------------------------------------------------

# 4. Intended users and workflows

  -----------------------------------------------------------------------
  Role                                Key actions
  ----------------------------------- -----------------------------------
  Sound designer / editor             Spot actions, choose sounds, AI
                                      timing, edit layered assets, prep
                                      stems and report.

  Foley artist                        Review cue list, audition scratch
                                      sounds, perform via MIDI/pad,
                                      record new takes, comp and replace.

  Picture editor                      Import/refresh cut and reference;
                                      review timecode-aligned rough
                                      premix in Resolve.

  Sound supervisor                    Assign work, audit missing cues and
                                      deliver complete editorial handoff.

  Finishing studio                    Reconform against picture, import
                                      stems or interchange, verify sync
                                      and read provenance/notes.
  -----------------------------------------------------------------------

# 5. End-to-end reference workflow

1.  Create project; set frame rate, audio sample rate, production start
    timecode, editorial version and output profile.

2.  Import picture reference (scene cut or full reel); detect/read
    source timecode when available, else user defines start TC; retain
    original media unmodified.

3.  Import script or scene breakdown from VC Film Studio; show script in
    left dock; generate candidate sound identities and scene cues from
    manually tagged script content.

4.  Watch picture with layered spotting markers; tap a key/MIDI pad to
    create point markers or in/out regions (e.g., footsteps start--stop;
    door handle rattle; glass smash).

5.  Open per-scene cue chart; review marker category, identity,
    start/end, priority, owner, notes and proposed library slot.

6.  Choose "Populate Placeholders": create organized tracks or reuse
    appropriate persistent identity tracks; place standard WAV scratch
    assets at exact cue positions.

7.  Optionally run AI Timing Assist on selected cue types to suggest
    onsets/offsets, repetitions and candidate alignment to on-screen
    actions; accept/edit individually or in batches.

8.  Replace placeholders with specific library sounds, record new Foley
    using input capture, or perform one-shots in sync using a keyboard,
    MIDI controller, touchscreen or pad.

9.  Adjust edits, fades, clip gains, effects, pans and automation;
    review score imported from VC Composer DAW at matching timecode;
    rough mix with inline faders or detachable mixer.

10. Review scene summary, gaps, overlaps, missing media, muted tracks
    and sync warnings; print landscape timecode-based cue report.

11. Export review mix, grouped stems, discrete assets, sound cue
    manifest, timeline interchange where supported, and a complete
    studio handoff package.

# 6. Screen layout and interaction design

## 6.1 Default workspace

-   Top: transport, timecode, frame-rate display, picture-version badge,
    snap/grid, markers and export action.

-   Left dock: script/scene breakdown with linked lines and searchable
    sound cues; collapsible sound identity tree, Foley library search
    and sound pack selector.

-   Center upper: video viewer with frame stepping and overlay options;
    selected marker/cue information.

-   Center lower: master marker/spotting timeline above audio lanes;
    vertically stack three to five or more named marker layers (no fixed
    limit).

-   Right inspector: selected cue and asset properties, timing, sound
    identity, source and replacement history, tags, notes and AI
    suggestion status.

-   Bottom: library browser/waveform preview, performance meters
    (CPU/RAM/disk/streaming) and timecode status.

-   Detachable full mixer for second monitor; inline track faders, pan
    and mute/solo remain available in timeline.

## 6.2 Fast spotting modes

-   Point marker: create a one-frame event ("glass breaks").

-   Range marker: press once for start and again for end, or drag a
    bounded region ("footsteps walking").

-   Repeated-event marker: create interval container for a series of
    strikes or steps; support inferred or tapped sub-events.

-   Live spotting: configurable hotkeys/MIDI pads mapped to common cue
    categories; maintain frame-accurate event timestamps.

-   Marker layers may represent Foley/footsteps, props, hard FX,
    atmospheres, special effects, dialogue notes, music or custom
    groups.

-   Marker edits update the linked cue records; ghost/preview before
    converting to audio instances.

# 7. Script and VC Film Studio integration

-   Film Studio script breakdown adds a Sound Design breakdown category
    directly on the script: proposed Foley, effects, ambience,
    environments, characters and notes.

-   Early script cues are conceptual and need no definitive blocking;
    film-cut-specific events can be appended after spotting picture.

-   Import scene IDs, character names, script line anchors, known sound
    cues and revision/version identifiers from Film Studio.

-   Left script panel supports jump to scene, click-highlight to linked
    cue/timecode and backlink from a timeline cue to script text.

-   Allow manual script import (PDF or structured text) as fallback;
    automatic extraction/scene matching is best-effort and must be
    editable.

-   Round-trip supported fields back to Film Studio without overwriting
    unrelated screenplay or breakdown data; resolve conflicts using
    stable IDs and user review.

# 8. Cue model, identities and track generation

## 8.1 Cue record required fields

  -----------------------------------------------------------------------
  Field                               Requirement
  ----------------------------------- -----------------------------------
  cue_id / scene_id                   Immutable unique IDs; valid
                                      project/reel references.

  identity_id                         Optional link to recurring sound
                                      identity; name and category
                                      visible.

  event_type                          Point / range / repeated event;
                                      track group and marker lane.

  start_tc / end_tc                   Frame-accurate reference plus
                                      internal sample position; end may
                                      equal start for point event.

  description / intent                Human-readable name and action,
                                      script reference, character,
                                      surface, prop.

  asset_ref / placeholder_ref         Media URI/hash, source version,
                                      license metadata and replacement
                                      state.

  timing_origin                       Manual / AI suggested / MIDI
                                      performed / recorded / imported;
                                      reviewer status.

  notes / status                      Assignments, editorial notes,
                                      priority,
                                      approved/missing/needs-recording
                                      flags.
  -----------------------------------------------------------------------

## 8.2 Sound identities across the film

A sound identity such as "Greg's boots" may appear in many scenes. The
system must offer a stable logical identity and reusable sonic palette
across the film, while supporting separate scene-local audio tracks,
playlists or lane views to avoid a physically unmanageable single track
over an entire feature. The UI should offer "show all Greg's boots cues"
and optionally route all instances through a shared bus. Default: reuse
the identity and its default sound assignment; never force physically
distant scene items onto a single global edit lane.

## 8.3 Auto-track builder

-   Convert approved markers to uniquely linked cue items; generate
    audio track(s) by sound identity/category according to
    user-selectable track policies.

-   Insert placeholder WAV at each cue. For ranged cues, loop,
    alternate, concatenate or fill with silence depending on asset/event
    semantics; flag any poor fit.

-   Preserve marker and source asset metadata even after replacement;
    provide "revert to placeholder" without losing custom takes.

-   Generate track labels such as "FOLEY • Greg • Boots", "PROPS • Door
    Handle", "BG • Birds", "FX • Glass".

-   Separate audio content from bus routing; organize folders by scene
    and department.

# 9. Sound library and optional downloadable packs

-   Optional starter sound pack downloaded separately: generic
    footsteps, doors, latches, glass, fabric, vehicles, wind, birds,
    rustling trees and common room/exterior ambience.

-   Support user-imported WAV libraries with indexing, tags, source
    path, sample rate, duration, channels, loudness previews and
    metadata.

-   Search and filter by category, character, surface, location,
    variation, microphone perspective, license and keywords.

-   Audition in sync with picture before replace; replacements preserve
    cue timing/identity and can optionally ripple linked uses after
    confirmation.

-   One identity may have variation pools with random/round-robin and
    user-locked choices to avoid repeated identical footsteps.

-   License and provenance manifest must travel with handoff; never
    assume third-party sounds are redistributable.

-   Keep downloaded packs and personal recordings separate from
    application binaries; missing assets trigger relink dialog.

# 10. AI-assisted spotting and timing

Core AI mode is user-directed. User selects which sounds exist in a
scene; AI attempts to align those selected cues to visual motion, onsets
and boundaries. General automatic "identify every sound" may be added
later but is not required for MVP.

-   Input: picture frames/video, selected cue categories, cue ranges,
    optional characters/props from script and user marker hints.

-   Output: proposed events with start/end sample position and source
    frame TC, confidence, reason and thumbnails (when allowed).

-   Examples: estimate visible footfalls in a marked walking range;
    align door handle touch/jiggle; identify glass-impact frame; propose
    ambience coverage over scene boundaries.

-   AI creates suggestions, never irreversible edits; show before/after
    preview, accept one/all, reject, drag-adjust and compare.

-   If occluded/off-screen, do not fabricate certainty; report low
    confidence and request manual spotting.

-   Cache suggestions per media fingerprint/picture version. Never
    auto-shift accepted edits when picture changes.

-   Support disabled/offline AI state with full marker-and-placeholder
    workflow intact.

-   No implicit upload of copyrighted footage; disclose cloud processing
    and offer explicit opt-in if cloud AI is later used.

# 11. Foley performance and recording

## 11.1 Perform-to-picture

-   Map audio one-shots/variants to computer keyboard, MIDI controller
    or touch-enabled pad grid.

-   While picture plays, capture taps as timecode-linked MIDI-like
    triggers; choose quantization off by default and support
    sub-frame/sample offsets.

-   Convert performed triggers into editable audio instances; retain
    original performance trigger data for re-rendering.

-   Support velocity, alternating foot selection, shoe/surface presets
    and round-robin samples.

-   Allow pre-roll, looping, variable-speed rehearsal, count-in and take
    versioning.

## 11.2 Live recording

-   Select audio interface/input, arm record track, monitor with
    adjustable latency/compensation, record WAV non-destructively.

-   Show placeholder as guide while recording and support muted guide
    signal to performers.

-   Multiple takes/playlists, slate/take metadata, waveform display,
    punch-in/out, trims and crossfades.

-   Store BWF-compatible time reference where feasible, with calibrated
    recording timestamps and explicit sync offset.

-   No requirement to synthesize new sound: real recorded Foley should
    be first-class.

# 12. Editing and mixing engine

-   Sample-accurate clip placement and frame-accurate video sync;
    slip/trim/split/crossfade, clip gain, fades, volume/pan automation,
    mute/solo and grouping.

-   Multichannel WAV and stereo/mono; plugin hosting (VST3 on
    Windows/macOS; AU on macOS if practical) as later-stage capability;
    built-in EQ, dynamics, filters, pitch and reverb initially.

-   Per-track quick fader and meters visible during playback; mixer
    floats/detaches to second display with channels, busses, inserts,
    sends, pan, automation and master controls.

-   Default department buses: Dialogue Reference, Foley, Hard FX,
    Backgrounds/Atmos, Design FX, Music/Score, Review Mix; users may
    customize.

-   Project autosave, undo/redo, media relink, waveform caching, proxy
    audio, freeze/render-in-place and background indexing.

-   Score lane imports timecoded exports from VC Composer DAW with start
    TC, frame rate, sample rate and tempo/bar markers when supplied;
    never assume music and picture start at 00:00:00:00.

-   CPU/RAM/disk/streaming meters and dropout warnings; audio/video
    tracks limited by available resources rather than licenses.

# 13. Timecode, frame-rate and picture versioning

-   Explicit support for project frame rates 23.976, 24, 25, 29.97
    DF/NDF and 30 (extendable); rational-rate calculations, not
    floating-point drift.

-   Represent timeline truth in integer audio sample positions plus
    rational frame mapping; maintain incoming source TC and sequence
    start TC.

-   Ingest picture with reference audio and embedded/start timecode when
    present; user verifies offsets.

-   Picture change workflow: import new version, store previous,
    compare/reconform with editorial change lists when provided, flag
    shifted cues for review.

-   Display absolute timeline and scene-relative offsets; scene
    boundaries and handles configurable.

-   Export all timestamps with frame rate and DF/NDF explicitly embedded
    in reports/manifests.

# 14. Reports and cue chart

Every scene has a printable landscape chart providing a complete
sound-by-time view. The chart complements, rather than replaces, the
sortable digital cue list.

-   Per-scene rows: cue ID, category, sound identity, sound description,
    start TC, end TC, duration, track, media filename,
    source/placeholder status, assigned operator, version and comments.

-   Timeline overview spans scene horizontally with distinct marker
    layers; label overlapping sounds; support dense-view pagination
    across landscape sheets.

-   At any playhead position, inspector lists all active sounds,
    including layered/overlapping clips and buses.

-   Exports: landscape PDF for print, CSV, JSON and optional XLSX;
    separate master cue inventory and one report per scene.

-   Include legend, picture version, film/reel/scene, sample rate, frame
    rate, date, page numbers and missing-media warnings.

-   Reports are reproducible from saved project state with deterministic
    cue sorting and stable IDs.

# 15. Professional handoff and export

## 15.1 Export profiles

  -----------------------------------------------------------------------
  Profile                             Required deliverables
  ----------------------------------- -----------------------------------
  Quick review                        Timecoded stereo rough mix WAV;
                                      optional reference movie with
                                      burn-in; short cue report.

  Resolve / Fairlight                 Consolidated aligned WAV stems
                                      and/or discrete tracks, source TC,
                                      cue sheet, media inventory;
                                      timeline interchange via validated
                                      supported path.

  Pro studio handoff                  Full consolidated media + handles,
                                      dry/wet variants where requested,
                                      routing map, all reports, source
                                      references, picture version,
                                      reconform notes and checksums.

  Archival / reopen                   Native editable project, media
                                      manifest, external-library
                                      references, version and dependency
                                      information.
  -----------------------------------------------------------------------

## 15.2 Interchange strategy

Treat interchange as multiple tiers: Tier 1 guaranteed aligned WAV/BWF
assets plus cue manifest; Tier 2 interchange such as AAF or other
Resolve-compatible timeline packages only after compatibility testing
against specified application versions. Do not claim that all routing,
plugins, fades, automation or markers round-trip into Fairlight without
tests. Where a format cannot faithfully retain data, render audio and
document the loss.

-   Support user-selected timecode starts, full-scene/reel/full-film
    range, sync leader, preroll, handles, naming conventions and folder
    hierarchy.

-   Export individual cues or consolidated track-aligned
    mono/stereo/multichannel WAV files at consistent sample rate/bit
    depth; include silence for alignment when appropriate.

-   Separate Foley, BG, FX, design, dialogue guide and music stems with
    user-editable bus mapping; avoid baking licensed temp references
    into final deliverables by default.

-   Optional track/clip interchange adapter architecture: AAF/other
    format feasibility spike first, then verified implementer for chosen
    Resolve/Fairlight target versions.

-   Exports preserve clip IDs, notes, timestamps, identity, library
    source, substitutions, markers and status in JSON/CSV even when
    interchange format loses them.

-   Generate delivery QA report: media linked, overlap/out-of-range,
    clipping, unsupported channels, sample-rate conversion, missing
    rights metadata and hash verification.

-   Export package includes README for receiving engineer, technical
    specs, picture reference, picture change history and
    contact/assignment fields (when entered).

## 15.3 Proposed directory structure

VC_Sound_Project/ Project.vcsd /Picture /Scripts /Audio/Imported
/Audio/LibraryRefs /Audio/RecordedFoley /Audio/Rendered /Cache /Reports
/Exports/\<date_profile\>/ /Manifests

# 16. Suggested implementation architecture (engineering recommendation)

  -----------------------------------------------------------------------
  Subsystem                           Implementation guidance
  ----------------------------------- -----------------------------------
  Desktop shell                       Cross-platform macOS + Windows;
                                      recommend native-capable shell with
                                      GPU video panel and detachable
                                      windows.

  Realtime audio                      Dedicated low-latency engine with
                                      deterministic transport, offline
                                      render and sample-clock timeline.
                                      Avoid making browser JavaScript
                                      responsible for hard realtime
                                      audio.

  Data storage                        Local SQLite or equivalent
                                      relational DB for metadata;
                                      immutable media file references,
                                      file hashes and migration version.

  Media services                      Video decoder/proxy service,
                                      waveform/spectrogram cache, robust
                                      FFmpeg or similarly licensed
                                      decoding pipeline.

  Interchange adapters                Modular exporters so AAF/Resolve
                                      handoff can be independently
                                      validated and versioned.

  AI assistance                       Optional local or opt-in remote
                                      inference worker; queue and
                                      cancellable suggestions; provenance
                                      capture.

  Film Studio/Composer                Versioned JSON or API boundary;
                                      local import/export works before
                                      any cloud connection exists.

  QA platform                         Automated sync roundtrip, sample
                                      accuracy, stress tests,
                                      representative edit and export
                                      fixtures.
  -----------------------------------------------------------------------

Hosting, licensing, cloud sync and monetization are not decided in this
spec. Prioritize a local-first desktop product and keep integrations
behind clear interfaces.

# 17. Proposed core data entities

Project, PictureVersion, Reel, Scene, ScriptReference, Character,
SoundIdentity, Cue, MarkerLane, Marker, Asset, AssetVersion,
PlaceholderPack, Track, Bus, ClipInstance, Take, PerformanceEvent,
AIProposal, AutomationPoint, ExportJob, HandoffManifest, DeliveryReport,
AuditEvent.

Relationships: Project → scenes; Scene → cues; Cue → identity, marker
and 0..n instances; ClipInstance → asset version and track; Track → bus;
every cue/clip binds to picture version or reconform mapping. All IDs
stable across import/export. Persist revisions/undo and original media
checksums.

# 18. Implementation phases and priorities

  -----------------------------------------------------------------------
  Phase                   Deliverable             Exit gate
  ----------------------- ----------------------- -----------------------
  P0 --- Foundation       Project model, import   A test scene plays in
                          picture/reference TC,   sync, saves and reopens
                          transport, audio        with exact positions.
                          engine, editable audio  
                          tracks, asset relink.   

  P1 --- Core spotting    Marker lanes, live      User spots a scene,
                          point/range spotting,   populates tracks and
                          scene cue chart,        replaces a cue without
                          identities, placeholder loss.
                          WAV packs and           
                          auto-track generation.  

  P2 --- Foley/editorial  MIDI/keyboard pads,     Foley artist records or
                          performances,           performs multiple takes
                          recording/playlists,    against picture.
                          editable fades, quick   
                          faders and detachable   
                          mixer.                  

  P3 --- Reports/handoff  Landscape scene         A receiving editor can
                          reports, CSV/JSON cue   relink and align all
                          manifest, consolidated  delivered assets from
                          aligned WAV/stems,      files and manifest.
                          checksum/QA package.    

  P4 --- VC integrations  Film Studio script      Script cues and score
                          breakdown/round-trip;   sync correctly without
                          Composer timecoded      destructive overwrites.
                          score ingest; picture   
                          version review.         

  P5 --- AI & interchange Opt-in timing           AI can be rejected;
                          proposals; validated    selected Resolve
                          Fairlight interchange   versions pass
                          adapter(s); advanced    documented import
                          reconform and batch     fidelity tests.
                          ops.                    
  -----------------------------------------------------------------------

# 19. Coding-agent work packages

12. Define domain schema, migration plan, stable IDs and project folder
    conventions; write test fixtures for sample and timecode conversion.

13. Prototype synchronized picture and audio transport with
    stop/play/frame stepping, sample-accurate edit placement and latency
    diagnostics.

14. Implement marker lanes and keyboard/MIDI spotting capture; persist
    point/range markers and scene cue lists.

15. Implement sound identities, pack importer, asset index, auditioner
    and non-destructive placeholder replacement.

16. Implement auto track generation, editable clips and track-folder/bus
    assignments.

17. Implement recording and performance-pad capture with deterministic
    render to audio clips.

18. Implement scene report renderer and CSV/JSON schemas, then WAV/stem
    delivery builder with QA and hash list.

19. Implement Film Studio and Composer import contracts and test
    roundtrip.

20. Spike and validate Resolve/Fairlight interchange against a pinned
    version; implement only verified subset and loss warnings.

21. Implement optional AI proposal pipeline with human approval;
    regression suite across every phase.

# 20. Acceptance scenarios (must automate where possible)

22. Import 24 fps reference scene at non-zero start TC, add a one-frame
    glass event, save/reopen; event remains on the exact frame.

23. Spot "Greg's boots" in three scenes; assign generic gravel steps,
    edit one scene to custom Foley; identity remains discoverable across
    all scenes and local override remains isolated.

24. Create footsteps range and manually tap eight steps via MIDI; all
    eight triggers appear as editable events within expected timing
    tolerance.

25. Insert repeated door-jiggle range then auto-populate placeholders;
    replace one instance with a new recorded take; original source
    persists in history.

26. Run AI alignment on marked footsteps, reject proposals; no existing
    clip time changes.

27. Import a score with non-zero picture start TC and detect a
    deliberate timecode mismatch before final export.

28. Produce landscape report for overlapping footsteps/birds/car; at any
    queried frame, report all active cue identities and exact in/out TC.

29. Export consolidated aligned mono Foley tracks and stereo ambience;
    re-import into a new blank timeline and verify waveform alignment
    against source within one sample for non-resampled paths.

30. Move imported library; project displays missing media and correctly
    relinks by hash without changing cues.

31. Deliver Fairlight package; receiving editor can reconstruct intended
    track layout from WAVs and cue manifest even if interchange adapter
    is unavailable.

# 21. Edge cases and nonfunctional requirements

-   Frame rates: 23.976 versus 24, DF/NDF, mixed source/frame rates and
    non-zero start TC.

-   Video revisions changing duration or scene boundaries;
    cut/insert/reorder review required.

-   Very short one-frame events; overlapping identical cues;
    negative/pre-roll handles; ambience loops with clean crossfades.

-   Missing/offline sound packs, different user library paths, invalid
    license, asset checksum mismatch.

-   Large sessions: hundreds of simultaneous clips and many soundtrack
    tracks; progressive loading, caching and dropout reporting.

-   High-latency devices, sample-rate mismatch, changed audio interface,
    microphone disconnect mid-take.

-   Interrupted recording, application crash, autosave recovery and
    interrupted export; no source media corruption.

-   Accessibility: keyboard-first spotting, high-contrast marker lanes,
    scalable UI and usable focus order.

-   Privacy: local footage by default; optional AI provider disclosure,
    consent and data retention control.

-   Deterministic renders, auditable output manifest, versioned schemas
    and testable performance budgets (to be measured in prototype).

# 22. Decisions to resolve during discovery

  -----------------------------------------------------------------------
  Decision                            Default proposal / evidence
                                      required
  ----------------------------------- -----------------------------------
  Desktop framework and audio engine  Choose through prototype
                                      demonstrating low-latency transport
                                      and dual-screen mixer.

  Native project format               Versioned project DB plus external
                                      media; open documented interchange
                                      manifests.

  Placeholder library licensing       Commission/own starter samples or
                                      secure explicit redistribution
                                      rights before bundling.

  Fairlight exchange                  Run AAF/other-format proof of
                                      concept; pin tested Resolve version
                                      and supported feature matrix.

  AI backend                          Start optional and opt-in; compare
                                      on-device and cloud
                                      cost/latency/privacy before
                                      selection.

  Recording hardware support          Start ASIO/WASAPI (Windows) and
                                      Core Audio (macOS), then validate
                                      device matrix.

  First release boundaries            P0--P3 form practical MVP; P4/P5
                                      may be separately staged once core
                                      is stable.
  -----------------------------------------------------------------------

# 23. Definition of done --- MVP

A user can import a picture scene with correct timecode; spot point and
range events in multiple marker lanes; generate a cue chart and
organized placeholder audio; reuse a named sound identity across scenes;
replace, perform and record audio; edit and mix it; produce a printable
landscape report with precise per-cue start/end timecodes; and export a
complete synchronized WAV/stem and JSON/CSV handoff package that can be
reconstructed by another editor. AI and proprietary interchange are
enhancements and must not block the baseline workflow.

Version control note: the above is the first formal baseline synthesized
from the VC Sound Design planning conversation. New requirements should
be tracked as numbered deltas with affected schema, UI, export contract
and acceptance tests.
