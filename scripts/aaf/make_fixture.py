#!/usr/bin/env python3
"""Write an AAF the way picture editors' systems lay one out, with pyaaf2 (an
independent AAF implementation), for the AAF import tests:

    python3 scripts/aaf/make_fixture.py OUT_DIR

OUT_DIR gets fixture.aaf, dialog.wav (embedded in the AAF too), music.wav
(linked, from a folder that no longer exists, so it must be found by name), and
fx.mxf: an OP1a MXF with two sound tracks (mono, then stereo; 3 channels), linked
as one 3-channel file, plus fx_ch3.wav, the channel the AAF uses (for checking).

The composition "Reel 1 v3" runs at 25 fps from 10:00:00:00:
- "DIA 1": filler, clip, transition (crossfade), clip; channel 1 of the
  embedded stereo dialog file;
- "MX": a clip with a constant Mono Audio Gain of 0.5, the whole track inside a
  Mono Audio Gain whose Amplitude varies 1 → 0.25 → 1, and a Mono Audio Pan of 0.75;
- "FX 3": channel 3 of fx.mxf (the right channel of its second track);
- "PAN": 50 frames of the music in a Mono Audio Pan sweeping (linear) 0 → 1 at
  frame 25 → 0.5 at the end;
- "STEP": the same, with Constant interpolation: pan 0.25 until frame 25, then 0.75;
- "CLIPGAIN": filler 10, then 40 frames of the music with a clip gain envelope
  (Mono Audio Gain on the clip, linear 1 → 0.25 → 1);
- "LEVEL": 50 frames of the music in a Stereo Audio Gain whose Level steps
  (Constant interpolation) from 0.5 to 8 (+18 dB) at frame 25;
- a picture-only track (ignored), a timecode track, and a descriptive
  marker "door knock" at frame 50.
"""
import math
import os
import struct
import sys

import aaf2
from aaf2.ama import create_media_link
from aaf2.rational import AAFRational

RATE = 48000
FPS = 25


def wav(path, chans, bits, frames, fn):
    data = bytearray()
    for i in range(frames):
        for c in range(chans):
            v = int(round(fn(i, c) * (2 ** (bits - 1) - 1)))
            data += struct.pack("<i", v)[: bits // 8]
    fmt = struct.pack("<HHIIHH", 1, chans, RATE, RATE * chans * bits // 8, chans * bits // 8, bits)
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(data)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data)
    return {
        "format": {"format_name": "wav", "format_long_name": "WAV / WAVE (Waveform Audio)", "filename": path},
        "streams": [{"codec_type": "audio", "codec_name": "pcm_s%dle" % bits, "sample_rate": str(RATE), "duration_ts": str(frames),
                     "channels": chans, "bit_rate": str(RATE * chans * bits), "sample_fmt": "s32"}],
    }


def klv(key, value):
    return bytes(key) + b"\x83" + len(value).to_bytes(3, "big") + value


def local_set(items):
    return b"".join(struct.pack(">HH", tag, len(v)) + v for tag, v in items)


def mxf(path, frames, tracks):
    """A minimal OP1a MXF: timeline tracks, linked wave descriptors, then
    frame-wrapped content packages (one BWF element per sound track, 24-bit)."""
    set_key = lambda kind: [0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, kind, 0x00]
    out = b""
    for n, (chans, _) in enumerate(tracks):
        tid, num = 2 + n, 0x16000101 + (len(tracks) << 16) + n
        out += klv(set_key(0x3b), local_set([(0x4801, struct.pack(">I", tid)), (0x4804, struct.pack(">I", num))]))
        out += klv(set_key(0x48), local_set([(0x3d03, struct.pack(">ii", RATE, 1)), (0x3d07, struct.pack(">I", chans)),
                                            (0x3d01, struct.pack(">I", 24)), (0x3006, struct.pack(">I", tid))]))
    per = RATE // FPS
    for start in range(0, frames, per):
        for n, (chans, fn) in enumerate(tracks):
            num = 0x16000101 + (len(tracks) << 16) + n
            data = b"".join(struct.pack("<i", int(round(fn(i, c) * (2 ** 23 - 1))))[:3]
                            for i in range(start, min(start + per, frames)) for c in range(chans))
            out += klv([0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01] + list(num.to_bytes(4, "big")), data)
    with open(path, "wb") as f:
        f.write(out)


def defs(f):
    amp = f.create.ParameterDef("e4962321-2267-11d3-8a4c-0050040ef7d2", "Amplitude", "", "Rational")
    f.dictionary.register_def(amp)
    gain = f.create.OperationDef("9d2ea894-0968-11d3-8a38-0050040ef7d2", "Mono Audio Gain", "")
    gain.media_kind = "sound"
    gain["NumberInputs"].value = 1
    gain["ParametersDefined"].append(amp)
    f.dictionary.register_def(gain)
    pan = f.create.ParameterDef("e4962322-2267-11d3-8a4c-0050040ef7d2", "Pan", "", "Rational")
    f.dictionary.register_def(pan)
    panop = f.create.OperationDef("9d2ea893-0968-11d3-8a38-0050040ef7d2", "Mono Audio Pan", "")
    panop.media_kind = "sound"
    panop["NumberInputs"].value = 1
    panop["ParametersDefined"].append(pan)
    f.dictionary.register_def(panop)
    dissolve = f.create.OperationDef("0c3bea41-fc05-11d2-8a29-0050040ef7d2", "Mono Audio Dissolve", "")
    dissolve.media_kind = "sound"
    dissolve["NumberInputs"].value = 2
    f.dictionary.register_def(dissolve)
    f.dictionary.register_def(f.create.InterpolationDef("5b6c85a4-0ede-11d3-80a9-006008143e6f", "LinearInterp", ""))
    f.dictionary.register_def(f.create.InterpolationDef("5b6c85a5-0ede-11d3-80a9-006008143e6f", "ConstantInterp", ""))
    # Stereo Audio Gain, with its level in "Level" (as some apps write gain)
    level = f.create.ParameterDef("e4962320-2267-11d3-8a4c-0050040ef7d2", "Level", "", "Rational")
    f.dictionary.register_def(level)
    sgain = f.create.OperationDef("9d2ea895-0968-11d3-8a38-0050040ef7d2", "Stereo Audio Gain", "")
    sgain.media_kind = "sound"
    sgain["NumberInputs"].value = 1
    sgain["ParametersDefined"].append(level)
    f.dictionary.register_def(sgain)
    return amp, gain, pan, panop, dissolve, level, sgain


def build(out):
    os.makedirs(out, exist_ok=True)
    dialog = os.path.join(out, "dialog.wav")
    music = os.path.join(out, "music.wav")
    secs = 4
    wav(dialog, 2, 16, RATE * secs, lambda i, c: (0.5 if c == 0 else 0.25) * math.sin(2 * math.pi * (440 if c == 0 else 660) * i / RATE))
    meta = wav(music, 1, 24, RATE * secs, lambda i, c: 0.4 * math.sin(2 * math.pi * 220 * i / RATE))
    # the MXF: track 1 mono, track 2 stereo (its right channel is the file's channel 3)
    fx_frames = RATE * 2
    right = lambda i, c: 0.3 * math.sin(2 * math.pi * 550 * i / RATE)
    mxf(os.path.join(out, "fx.mxf"), fx_frames, [
        (1, lambda i, c: 0.2 * math.sin(2 * math.pi * 110 * i / RATE)),
        (2, lambda i, c: right(i, c) if c == 1 else -0.1),
    ])
    wav(os.path.join(out, "fx_ch3.wav"), 1, 24, fx_frames, right)
    # AMA metadata for a 3-channel file of the same length (the link then points at the MXF)
    fx_ref = os.path.join(out, "fx_ref.wav")
    fx_meta = wav(fx_ref, 3, 24, fx_frames, lambda i, c: 0.0)

    with aaf2.open(os.path.join(out, "fixture.aaf"), "w") as f:
        amp, gain, pan, panop, dissolve, level, sgain = defs(f)

        # embedded: the dialog's PCM inside the AAF (master at the video rate)
        dmaster = f.create.MasterMob("dialog")
        f.content.mobs.append(dmaster)
        dmaster.import_audio_essence(dialog, FPS)

        # linked: AMA-style master → file → tape, its location since moved
        mmaster, msource, _ = create_media_link(f, music, meta)
        msource.descriptor["Locator"][0]["URLString"].value = "file:///Volumes/Gone/Audio%20Files/music.wav"

        xmaster, xsource, _ = create_media_link(f, fx_ref, fx_meta)
        xsource.descriptor["Locator"][0]["URLString"].value = "file://" + os.path.abspath(os.path.join(out, "fx.mxf"))

        comp = f.create.CompositionMob("Reel 1 v3")
        f.content.mobs.append(comp)

        tc = comp.create_timeline_slot(FPS)
        tc.segment = f.create.Timecode(FPS, False)
        tc.segment.start = 10 * 3600 * FPS
        tc.segment.length = 250

        pic = comp.create_timeline_slot(FPS)
        pic.name = "V1"
        pic.segment = f.create.Sequence(media_kind="picture")
        pic.segment.components.append(f.create.Filler("picture", 100))

        # DIA 1: filler 25, clip 50 (from 0), crossfade 10, clip 40 (from 60)
        a1 = comp.create_sound_slot(FPS)
        a1.name = "DIA 1"
        seq = a1.segment
        seq.components.append(f.create.Filler("sound", 25))
        seq.components.append(dmaster.create_source_clip(1, start=0, length=50))
        t = f.create.Transition("sound", 10)
        t["OperationGroup"].value = f.create.OperationGroup(dissolve, 10, "sound")
        t["CutPoint"].value = 5
        seq.components.append(t)
        seq.components.append(dmaster.create_source_clip(1, start=60, length=40))

        # MX: clip gain 0.5 on a clip of the linked music, track gain 1 → 0.25 → 1, pan 0.75
        a2 = comp.create_sound_slot(FPS)
        a2.name = "MX"
        inner = f.create.Sequence(media_kind="sound")
        og = f.create.OperationGroup(gain, 75, "sound")
        og["InputSegments"].append(mmaster.create_source_clip(1, start=25 * RATE // FPS, length=75 * RATE // FPS))
        og["Parameters"].append(f.create.ConstantValue(amp, AAFRational(1, 2)))
        # the master runs at the sample rate; the clip here is in frames
        og.segments[0].length = 75
        og.segments[0].start = 25 * RATE // FPS
        inner.components.append(og)
        inner.components.append(f.create.Filler("sound", 25))
        auto = f.create.OperationGroup(gain, 100, "sound")
        auto["InputSegments"].append(inner)
        vv = f.create.VaryingValue(amp, "LinearInterp")
        for tm, v in ((0, 1), (0.5, 0.25), (1, 1)):
            cp = f.create.ControlPoint()
            cp["Time"].value = AAFRational(tm)
            cp["Value"].value = AAFRational(v)
            vv["PointList"].append(cp)
        auto["Parameters"].append(vv)
        po = f.create.OperationGroup(panop, 100, "sound")
        po["InputSegments"].append(auto)
        po["Parameters"].append(f.create.ConstantValue(pan, AAFRational(3, 4)))
        a2.segment = po

        # FX 3: channel 3 of the MXF, 50 frames from its start
        a3 = comp.create_sound_slot(FPS)
        a3.name = "FX 3"
        clip = xmaster.create_source_clip(3, start=0, length=50 * RATE // FPS)
        clip.length = 50
        a3.segment.components.append(clip)

        # PAN / STEP: pan automation, linear and stepped
        for name, interp, keys in (("PAN", "LinearInterp", ((0, 0), (0.5, 1), (1, 0.5))),
                                   ("STEP", "ConstantInterp", ((0, 0.25), (0.5, 0.75)))):
            slot = comp.create_sound_slot(FPS)
            slot.name = name
            seq = f.create.Sequence(media_kind="sound")
            c = mmaster.create_source_clip(1, start=0, length=50 * RATE // FPS)
            c.length = 50
            seq.components.append(c)
            seq.length = 50
            po = f.create.OperationGroup(panop, 50, "sound")
            po["InputSegments"].append(seq)
            vv = f.create.VaryingValue(pan, interp)
            for tm, v in keys:
                cp = f.create.ControlPoint()
                cp["Time"].value = AAFRational(tm)
                cp["Value"].value = AAFRational(v)
                vv["PointList"].append(cp)
            po["Parameters"].append(vv)
            slot.segment = po

        def varying(param, interp, keys):
            vv = f.create.VaryingValue(param, interp)
            for tm, v in keys:
                cp = f.create.ControlPoint()
                cp["Time"].value = AAFRational(tm)
                cp["Value"].value = AAFRational(v)
                vv["PointList"].append(cp)
            return vv

        # CLIPGAIN: a clip gain envelope on a clip 10 frames in
        slot = comp.create_sound_slot(FPS)
        slot.name = "CLIPGAIN"
        seq = f.create.Sequence(media_kind="sound")
        seq.components.append(f.create.Filler("sound", 10))
        c = mmaster.create_source_clip(1, start=0, length=40 * RATE // FPS)
        c.length = 40
        cg = f.create.OperationGroup(gain, 40, "sound")
        cg["InputSegments"].append(c)
        cg["Parameters"].append(varying(amp, "LinearInterp", ((0, 1), (0.5, 0.25), (1, 1))))
        seq.components.append(cg)
        seq.length = 50
        slot.segment = seq

        # LEVEL: Stereo Audio Gain, Level stepping from 0.5 to 8 at the midpoint
        slot = comp.create_sound_slot(FPS)
        slot.name = "LEVEL"
        seq = f.create.Sequence(media_kind="sound")
        c = mmaster.create_source_clip(1, start=0, length=50 * RATE // FPS)
        c.length = 50
        seq.components.append(c)
        seq.length = 50
        sg = f.create.OperationGroup(sgain, 50, "sound")
        sg["InputSegments"].append(seq)
        sg["Parameters"].append(varying(level, "ConstantInterp", ((0, 0.5), (0.5, 8))))
        slot.segment = sg

        ev = f.create.EventMobSlot()
        ev["SlotID"].value = 50
        ev["EditRate"].value = FPS
        ev["SlotName"].value = "Markers"
        ev["PhysicalTrackNumber"].value = 1
        eseq = f.create.Sequence(media_kind="DescriptiveMetadata")
        mk = f.create.DescriptiveMarker()
        mk["DataDefinition"].value = f.dictionary.lookup_datadef("DescriptiveMetadata")
        mk["Position"].value = 50
        mk["Comment"].value = "door knock"
        # the tracks it describes, as Media Composer writes them (readers such as the
        # OpenTimelineIO AAF adapter need them)
        mk["DescribedSlots"].value = [a1.slot_id]
        eseq.components.append(mk)
        ev.segment = eseq
        comp.slots.append(ev)


if __name__ == "__main__":
    build(sys.argv[1])
    os.remove(os.path.join(sys.argv[1], "fx_ref.wav"))
