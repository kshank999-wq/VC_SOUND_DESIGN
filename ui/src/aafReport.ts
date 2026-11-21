// AAF and OTIO import results, as the engine reports them (aaf.import, otio.import).

export type AafReport = {
  composition: string; start_tc: string; tracks: string[]; stereo_tracks: number; clips: number; markers: number;
  media_linked: number; media_written: number; missing_media: string[]; notes: string[];
};

/** OTIO import results: the same, with the timeline's name. */
export type OtioReport = Omit<AafReport, "composition"> & { timeline: string };

/** What an AAF (or OTIO) import brought in, and what it couldn't. */
export function aafSummary(r: AafReport, mediaDir = "Audio/Imported/AAF"): string[] {
  const lines = [
    `"${r.composition}" at ${r.start_tc}: ${r.tracks.length} track${r.tracks.length === 1 ? "" : "s"}${r.stereo_tracks ? ` (${r.stereo_tracks} stereo)` : ""}, ${r.clips} clip${r.clips === 1 ? "" : "s"}, ${r.markers} marker${r.markers === 1 ? "" : "s"}`,
    `Media: ${r.media_linked} linked in place, ${r.media_written} written to ${mediaDir}`,
  ];
  if (r.missing_media.length) lines.push("", "Missing media (clips left out):", ...r.missing_media.map((m) => "  " + m));
  if (r.notes.length) lines.push("", "Notes:", ...r.notes.map((n) => "  " + n));
  return lines;
}

export function otioSummary(r: OtioReport): string[] {
  return aafSummary({ ...r, composition: r.timeline }, "Audio/Imported/OTIO");
}
