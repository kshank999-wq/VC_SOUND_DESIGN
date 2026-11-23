#!/usr/bin/env python3
"""Write an OTIO timeline the way editors' tools lay one out, with the
OpenTimelineIO library, for the OTIO import tests:

    python3 scripts/otio/make_fixture.py OUT_DIR

OUT_DIR gets fixture.otio, media/dialog.wav (stereo, 16-bit; its available range
starts at 10:00:00:00, as editors give BWF media) and music.aiff (big-endian
AIFF, linked from a folder that no longer exists, so found by name).

"Reel 2" runs at 25 fps from 10:00:00:00:
- V1: a video track (ignored);
- DIA 1: gap 25, fade in 5, clip A (dialog from 10, 50 long), dissolve 5/5,
  clip B (dialog from 70, 25 long), gap 10, clip C (disabled, 10 long); a
  marker on clip A at media frame 20 (timeline frame 35); on clip B a range
  from media frame 60, 20 long (B uses 70–95: trimmed to timeline 75–85) and
  one at 30 (unused media: left out); on the gap a range at 2, 4 long (102–106);
- MX: music 25 long, then a nested stack (range 10–30) holding track "inner":
  gap 5, dialog 40 long — so frames 5–25 of the dialog play at 25–45; a range
  marker on the stack at its frame 15, 5 long (timeline 30–35);
- Missing: a clip with a missing reference;
- timeline markers "door knock" at frame 50 (RED; the clip's marker is CYAN)
  and "ambience" from 60, 24 long.
"""
import math
import os
import struct
import sys

import opentimelineio as otio

RATE = 48000
FPS = 25
TC = 10 * 3600 * FPS
RT = otio.opentime.RationalTime
TR = otio.opentime.TimeRange


def f(n):
    return RT(n, FPS)


def wav(path, chans, frames, fn):
    data = b"".join(struct.pack("<h", int(round(fn(i, c) * 32767))) for i in range(frames) for c in range(chans))
    fmt = struct.pack("<HHIIHH", 1, chans, RATE, RATE * chans * 2, chans * 2, 16)
    with open(path, "wb") as fh:
        fh.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(data)) + b"WAVE")
        fh.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data)


def aiff(path, frames, fn):
    data = b"".join(struct.pack(">h", int(round(fn(i) * 32767))) for i in range(frames))
    # 48000 as an 80-bit extended float
    rate = bytes([0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0])
    comm = struct.pack(">hIh", 1, frames, 16) + rate
    ssnd = struct.pack(">II", 0, 0) + data
    body = b"AIFF" + b"COMM" + struct.pack(">I", len(comm)) + comm + b"SSND" + struct.pack(">I", len(ssnd)) + ssnd
    with open(path, "wb") as fh:
        fh.write(b"FORM" + struct.pack(">I", len(body)) + body)


def build(out):
    os.makedirs(os.path.join(out, "media"), exist_ok=True)
    wav(os.path.join(out, "media", "dialog.wav"), 2, RATE * 4,
        lambda i, c: (0.5 if c == 0 else 0.25) * math.sin(2 * math.pi * (440 if c == 0 else 660) * i / RATE))
    aiff(os.path.join(out, "music.aiff"), RATE * 2, lambda i: 0.4 * math.sin(2 * math.pi * 220 * i / RATE))

    dialog = lambda: otio.schema.ExternalReference("media/dialog.wav", TR(f(TC), f(100)))
    music = otio.schema.ExternalReference("file:///Volumes/Gone/Audio%20Files/music.aiff", TR(f(0), f(50)))

    tl = otio.schema.Timeline("Reel 2", global_start_time=f(TC))
    v1 = otio.schema.Track("V1", kind=otio.schema.TrackKind.Video)
    v1.append(otio.schema.Gap(source_range=TR(f(0), f(100))))
    tl.tracks.append(v1)

    dia = otio.schema.Track("DIA 1", kind=otio.schema.TrackKind.Audio)
    dia.append(otio.schema.Gap(source_range=TR(f(0), f(25))))
    dia.append(otio.schema.Transition(transition_type="SMPTE_Dissolve", in_offset=f(0), out_offset=f(5)))
    a = otio.schema.Clip("A", media_reference=dialog(), source_range=TR(f(TC + 10), f(50)))
    a.markers.append(otio.schema.Marker("line", TR(f(TC + 20), f(0)), color="CYAN", comment="pickup"))
    dia.append(a)
    dia.append(otio.schema.Transition(transition_type="SMPTE_Dissolve", in_offset=f(5), out_offset=f(5)))
    b = otio.schema.Clip("B", media_reference=dialog(), source_range=TR(f(TC + 70), f(25)))
    # a range starting before B's trimmed head (trimmed to it), and one in media B doesn't use
    b.markers.append(otio.schema.Marker("breath", TR(f(TC + 60), f(20)), color="YELLOW"))
    b.markers.append(otio.schema.Marker("unused take", TR(f(TC + 30), f(5))))
    dia.append(b)
    gap = otio.schema.Gap(source_range=TR(f(0), f(10)))
    gap.markers.append(otio.schema.Marker("room tone", TR(f(2), f(4)), color="GREEN"))
    dia.append(gap)
    c = otio.schema.Clip("C", media_reference=dialog(), source_range=TR(f(TC), f(10)))
    c.enabled = False
    dia.append(c)
    tl.tracks.append(dia)

    mx = otio.schema.Track("MX", kind=otio.schema.TrackKind.Audio)
    mx.append(otio.schema.Clip("music", media_reference=music, source_range=TR(f(0), f(25))))
    nested = otio.schema.Stack("compound", source_range=TR(f(10), f(20)))
    inner = otio.schema.Track("inner", kind=otio.schema.TrackKind.Audio)
    inner.append(otio.schema.Gap(source_range=TR(f(0), f(5))))
    inner.append(otio.schema.Clip("D", media_reference=dialog(), source_range=TR(f(TC), f(40))))
    nested.append(inner)
    nested.markers.append(otio.schema.Marker("swell", TR(f(15), f(5)), color="PURPLE"))
    mx.append(nested)
    tl.tracks.append(mx)

    miss = otio.schema.Track("Missing", kind=otio.schema.TrackKind.Audio)
    miss.append(otio.schema.Clip("lost", media_reference=otio.schema.MissingReference(), source_range=TR(f(0), f(10))))
    tl.tracks.append(miss)

    tl.tracks.markers.append(otio.schema.Marker("door knock", TR(f(50), f(0)), color="RED"))
    tl.tracks.markers.append(otio.schema.Marker("ambience", TR(f(60), f(24)), color="BLUE"))
    otio.adapters.write_to_file(tl, os.path.join(out, "fixture.otio"))


if __name__ == "__main__":
    build(sys.argv[1])
