#!/usr/bin/env python3
"""Write an AAF the way picture editors' systems lay one out, with pyaaf2 (an
independent AAF implementation), for the AAF import tests:

    python3 scripts/aaf/make_fixture.py OUT_DIR

OUT_DIR gets fixture.aaf, dialog.wav (embedded in the AAF too) and music.wav
(linked, from a folder that no longer exists, so it must be found by name).

The composition "Reel 1 v3" runs at 25 fps from 10:00:00:00:
- "DIA 1": filler, clip, transition (crossfade), clip; channel 1 of the
  embedded stereo dialog file;
- "MX": a clip with a constant Mono Audio Gain of 0.5, the whole track inside a
  Mono Audio Gain whose Amplitude varies 1 → 0.25 → 1, and a Mono Audio Pan of 0.75;
- "Music" a picture-only track (ignored), a timecode track, and a descriptive
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
    return amp, gain, pan, panop, dissolve


def build(out):
    os.makedirs(out, exist_ok=True)
    dialog = os.path.join(out, "dialog.wav")
    music = os.path.join(out, "music.wav")
    secs = 4
    wav(dialog, 2, 16, RATE * secs, lambda i, c: (0.5 if c == 0 else 0.25) * math.sin(2 * math.pi * (440 if c == 0 else 660) * i / RATE))
    meta = wav(music, 1, 24, RATE * secs, lambda i, c: 0.4 * math.sin(2 * math.pi * 220 * i / RATE))

    with aaf2.open(os.path.join(out, "fixture.aaf"), "w") as f:
        amp, gain, pan, panop, dissolve = defs(f)

        # embedded: the dialog's PCM inside the AAF (master at the video rate)
        dmaster = f.create.MasterMob("dialog")
        f.content.mobs.append(dmaster)
        dmaster.import_audio_essence(dialog, FPS)

        # linked: AMA-style master → file → tape, its location since moved
        mmaster, msource, _ = create_media_link(f, music, meta)
        msource.descriptor["Locator"][0]["URLString"].value = "file:///Volumes/Gone/Audio%20Files/music.wav"

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

        ev = f.create.EventMobSlot()
        ev["SlotID"].value = 50
        ev["EditRate"].value = FPS
        ev["SlotName"].value = "Markers"
        eseq = f.create.Sequence(media_kind="DescriptiveMetadata")
        mk = f.create.DescriptiveMarker()
        mk["DataDefinition"].value = f.dictionary.lookup_datadef("DescriptiveMetadata")
        mk["Position"].value = 50
        mk["Comment"].value = "door knock"
        eseq.components.append(mk)
        ev.segment = eseq
        comp.slots.append(ev)


if __name__ == "__main__":
    build(sys.argv[1])
