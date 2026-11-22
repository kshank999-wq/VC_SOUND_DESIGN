// AAF and OTIO import results, as the engine reports them (aaf.import, otio.import).

export type AafReport = {
  composition: string; start_tc: string; tracks: string[]; stereo_tracks: number; clips: number; markers: number;
  media_linked: number; media_written: number; missing_media: string[]; notes: string[];
};

/** OTIO import results: the same, with the timeline's name. */
export type OtioReport = Omit<AafReport, "composition"> & {
  timeline: string; inserts?: number; plugins?: number; automated_parameters?: number;
  bus_levels?: string; buses_restored?: number; buses_created?: number;
};

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
  const lines = aafSummary({ ...r, composition: r.timeline }, "Audio/Imported/OTIO");
  if (r.inserts) {
    const auto = r.automated_parameters ? `, ${r.automated_parameters} automated parameter${r.automated_parameters === 1 ? "" : "s"}` : "";
    lines.splice(2, 0, `Inserts: ${r.inserts} (${r.plugins ?? 0} plugin${r.plugins === 1 ? "" : "s"}${auto})`);
  }
  if (r.bus_levels === "buses" && r.buses_restored) {
    const made = r.buses_created ? `, ${r.buses_created} created` : "";
    lines.splice(2, 0, `Buses: ${r.buses_restored} restored with their automation${made}`);
  }
  return lines;
}
