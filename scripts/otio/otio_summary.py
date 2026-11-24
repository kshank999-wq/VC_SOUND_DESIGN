#!/usr/bin/env python3
"""Read an OTIO file with the OpenTimelineIO library and print a JSON summary
of its timeline, times in samples. Used by the tests to check what VC Sound
Design writes.

    python3 -m pip install opentimelineio==0.18.1
    python3 scripts/otio/otio_summary.py file.otio [sample_rate]
"""
import json
import sys
from urllib.parse import urlparse
from urllib.request import url2pathname

import opentimelineio as otio


def effects(item):
    """Effects as names and their AAF-style level parameters."""
    out = []
    for e in item.effects:
        meta = json.loads(otio.core.serialize_json_to_string(e.metadata))
        out.append({"name": e.name, "effect_name": e.effect_name,
                    "parameters": meta.get("AAF", {}).get("Parameters", {})})
    return out


def main(path, sr):
    tl = otio.adapters.read_from_file(path)
    s = lambda t: int(round(t.rescaled_to(sr).value))
    out = {
        "name": tl.name,
        "global_start": [tl.global_start_time.value, tl.global_start_time.rate],
        "duration": s(tl.duration()),
        "metadata": json.loads(otio.core.serialize_json_to_string(tl.metadata)),
        "markers": [{"name": m.name, "comment": m.comment, "color": m.color, "start": s(m.marked_range.start_time),
                     "duration": s(m.marked_range.duration)} for m in tl.tracks.markers],
        "tracks": [],
    }
    for track in tl.tracks:
        t = {"name": track.name, "kind": track.kind, "metadata": json.loads(otio.core.serialize_json_to_string(track.metadata)),
             "duration": s(track.duration()), "effects": effects(track), "items": []}
        for item in track:
            if isinstance(item, otio.schema.Transition):
                t["items"].append({"kind": "Transition", "type": item.transition_type, "in": s(item.in_offset), "out": s(item.out_offset),
                                   "metadata": json.loads(otio.core.serialize_json_to_string(item.metadata))})
                continue
            r = track.range_of_child(item)
            d = {"kind": type(item).__name__, "start": s(r.start_time), "duration": s(r.duration)}
            if isinstance(item, otio.schema.Clip):
                ref = item.media_reference
                d["name"] = item.name
                d["source"] = s(item.source_range.start_time)
                d["url"] = ref.target_url
                # url2pathname: file:///C:/x → C:\x on Windows, unquoted as it is elsewhere
                d["file"] = url2pathname(urlparse(ref.target_url).path)
                d["available"] = s(ref.available_range.duration)
                d["metadata"] = json.loads(otio.core.serialize_json_to_string(item.metadata))
                d["effects"] = effects(item)
            t["items"].append(d)
        out["tracks"].append(t)
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 48000)
