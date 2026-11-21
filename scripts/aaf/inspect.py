#!/usr/bin/env python3
"""Read an AAF with pyaaf2 (an independent implementation) and print a JSON
summary of its timeline. Used by the tests to check what VC Sound Design writes.

    python3 scripts/aaf/inspect.py file.aaf
"""
import json
import sys

import aaf2


def rational(r):
    return [r.numerator, r.denominator]


def media_of(mob, slot_id):
    """Follow master → file source → tape mobs; return the linked file."""
    out = {"master": mob.name, "master_slot": slot_id}
    slot = next(s for s in mob.slots if s.slot_id == slot_id)
    clip = slot.segment.components[0] if slot.segment.classdef.class_name == "Sequence" else slot.segment
    file_mob = clip.mob
    desc = file_mob.descriptor
    out["file_mob"] = file_mob.name
    out["descriptor"] = desc.classdef.class_name
    out["urls"] = [l["URLString"].value for l in desc["Locator"]]
    out["sample_rate"] = rational(desc["SampleRate"].value)
    out["length"] = desc["Length"].value
    out["summary_riff"] = bytes(desc["Summary"].value[:4]).decode("ascii")
    fslot = next(s for s in file_mob.slots if s.slot_id == clip.slot_id)
    tape = fslot.segment.components[0].mob
    out["tape_mob"] = tape.name
    out["tape_descriptor"] = tape.descriptor.classdef.class_name
    return out


def component(c):
    kind = c.classdef.class_name
    d = {"kind": kind, "length": c.length}
    if kind == "OperationGroup":
        d["operation"] = c.operation.name
        params = [p for _, p in c["Parameters"].items()]
        d["gain"] = float(params[0].value) if params else None
        inner = component(c.segments[0])
        inner["gain"] = d["gain"]
        inner["wrapped_in"] = d["operation"]
        return inner
    if kind == "SourceClip":
        d["start"] = c.start
        d["fade_in"] = c["FadeInLength"].value if "FadeInLength" in c else 0
        d["fade_out"] = c["FadeOutLength"].value if "FadeOutLength" in c else 0
        d["media"] = media_of(c.mob, c.slot_id)
    return d


def walk_all(obj, seen):
    """Read every property of every object reachable from `obj` (catches broken references)."""
    if id(obj) in seen:
        return 0
    seen.add(id(obj))
    n = 1
    for p in obj.properties():
        v = p.value
        items = v if isinstance(v, list) else (list(v.values()) if isinstance(v, dict) else [v])
        if p.format in (0x3A,):
            items = [o for _, o in p.items()]
        for x in items:
            if isinstance(x, aaf2.core.AAFObject):
                n += walk_all(x, seen)
    return n


def main(path):
    f = aaf2.open(path, "r")
    objects = walk_all(f.content, set()) + walk_all(f.header, set())
    comps = list(f.content.compositionmobs())
    assert len(comps) == 1, "one composition"
    comp = comps[0]
    out = {
        "product": f.header["IdentificationList"][0]["ProductName"].value,
        "composition": comp.name,
        "mobs": len(f.content.mobs),
        "objects_read": objects,
        "timecode": None,
        "tracks": [],
        "markers": [],
    }
    for slot in comp.slots:
        seg = slot.segment
        if seg.classdef.class_name == "Timecode":
            out["timecode"] = {"start": seg.start, "fps": seg.fps, "drop": seg.drop, "edit_rate": rational(slot.edit_rate), "length": seg.length}
        elif slot.classdef.class_name == "EventMobSlot":
            for m in seg.components:
                out["markers"].append({"position": m["Position"].value, "comment": m["Comment"].value, "kind": m.classdef.class_name})
        else:
            comps_ = [component(c) for c in seg.components]
            assert sum(c["length"] for c in comps_) == seg.length, "sequence length is the sum of its parts"
            out["tracks"].append({
                "name": slot.name,
                "media_kind": seg.media_kind,
                "edit_rate": rational(slot.edit_rate),
                "track": slot["PhysicalTrackNumber"].value if "PhysicalTrackNumber" in slot else None,
                "length": seg.length,
                "components": comps_,
            })
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main(sys.argv[1])
