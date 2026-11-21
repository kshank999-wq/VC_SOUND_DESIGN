#!/usr/bin/env python3
"""Generate crates/vcsd-core/assets/aaf_template.aaf with pyaaf2.

VC Sound Design writes AAF itself (crates/vcsd-core/src/aaf.rs) but takes the
fixed parts of an AAF file from this template: the MetaDictionary (every class
and type definition), the Dictionary (data, container, operation and parameter
definitions) and the shared weak-reference table. The template's dummy content
uses every kind of object the writer emits, so all the weak-reference targets
it needs are registered; the writer then replaces the content entirely.

    python3 -m pip install pyaaf2==1.7.1
    python3 scripts/aaf/make_template.py [--dump]
"""
import os
import struct
import sys
import tempfile

import aaf2
from aaf2.ama import create_media_link

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "crates", "vcsd-core", "assets", "aaf_template.aaf")


def wav(path, channels=2, rate=48000, frames=4800):
    data = b"\0" * (frames * channels * 3)
    fmt = struct.pack("<HHIIHH", 1, channels, rate, rate * channels * 3, channels * 3, 24)
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(data)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data)
    return {
        "format": {"format_name": "wav", "format_long_name": "WAV / WAVE (Waveform Audio)"},
        "streams": [{"codec_type": "audio", "codec_name": "pcm_s24le", "sample_rate": str(rate), "duration_ts": str(frames),
                     "channels": channels, "bit_rate": str(rate * channels * 24), "sample_fmt": "s32"}],
    }


def build(path):
    tmp = tempfile.mkdtemp()
    src = os.path.join(tmp, "source.wav")
    meta = wav(src)
    with aaf2.open(path, "w") as f:
        ident = f.header["IdentificationList"][0]
        ident["CompanyName"].value = "VC Studio"
        ident["ProductName"].value = "VC Sound Design"
        ident["ProductVersionString"].value = "0"
        for name in ("Sound", "Timecode", "DescriptiveMetadata", "Picture"):
            f.dictionary.lookup_datadef(name)
        # clip gain: the standard "Mono Audio Gain" effect with its Amplitude parameter
        amp = f.create.ParameterDef("e4962321-2267-11d3-8a4c-0050040ef7d2", "Amplitude", "", "Rational")
        f.dictionary.register_def(amp)
        gain = f.create.OperationDef("9d2ea894-0968-11d3-8a38-0050040ef7d2", "Mono Audio Gain", "")
        gain.media_kind = "sound"
        gain["NumberInputs"].value = 1
        gain["IsTimeWarp"].value = False
        gain["Bypass"].value = 1
        gain["ParametersDefined"].append(amp)
        f.dictionary.register_def(gain)

        master = create_media_link(f, src, meta)[0]  # (master, source, tape)
        comp = f.create.CompositionMob("template")
        f.content.mobs.append(comp)
        rate = 48000

        tc_slot = comp.create_timeline_slot(24)
        tc_slot.segment = f.create.Timecode(24, False)
        tc_slot.segment.start = 86400
        tc_slot.segment.length = 240

        for ch in (1, 2):
            slot = comp.create_sound_slot(rate)
            slot["PhysicalTrackNumber"].value = ch
            seq = f.create.Sequence(media_kind="sound")
            seq.components.append(f.create.Filler("sound", 100))
            clip = master.create_source_clip(ch, start=10, length=1000)
            clip["FadeInLength"].value = 48
            clip["FadeInType"].value = "FadeLinearAmp"
            clip["FadeOutLength"].value = 96
            clip["FadeOutType"].value = "FadeLinearAmp"
            og = f.create.OperationGroup(gain, 1000, "sound")
            og["InputSegments"].append(clip)
            p = f.create.ConstantValue(amp, aaf2.rational.AAFRational(1, 2))
            og["Parameters"].append(p)
            seq.components.append(og)
            seq.length = 1100
            slot.segment = seq

        ev = f.create.EventMobSlot()
        ev["SlotID"].value = 100
        ev["EditRate"].value = rate
        ev["SlotName"].value = "Cues"
        eseq = f.create.Sequence(media_kind="DescriptiveMetadata")
        mk = f.create.CommentMarker()
        mk["DataDefinition"].value = f.dictionary.lookup_datadef("DescriptiveMetadata")
        mk["Position"].value = 100
        mk["Comment"].value = "cue"
        eseq.components.append(mk)
        ev.segment = eseq
        comp.slots.append(ev)


def dump(path):
    f = aaf2.open(path, "r")

    def one(o, d=0):
        pad = "  " * d
        print(pad + "# " + o.classdef.class_name, str(o.classdef.auid))
        for pid, p in o.property_entries.items():
            n = p.propertydef.property_name if p.propertydef else "?"
            data = p.data.hex() if p.data is not None and len(p.data) < 90 else len(p.data or b"")
            print(pad + "  %04x %-26s fmt=%02x %s" % (pid, n, p.format, data))
            if p.format in (0x22,):
                one(p.value, d + 1)
            elif p.format in (0x32, 0x3A):
                for x in p.value:
                    one(x, d + 1)

    for m in f.content.mobs:
        one(m)
    one(f.header["IdentificationList"][0])
    print("referenced properties:", f.cfb.find("/referenced properties").open("r").read().hex())
    wr = f.weakref_table if hasattr(f, "weakref_table") else None
    if wr:
        for i, path in enumerate(wr):
            print("weakref", i, path)


if __name__ == "__main__":
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    build(OUT)
    print("wrote", os.path.normpath(OUT), os.path.getsize(OUT), "bytes")
    if "--dump" in sys.argv:
        dump(OUT)
