// AAF and OTIO import results, as the engine reports them (aaf.import, otio.import).

export type AafReport = {
  composition: string; start_tc: string; tracks: string[]; stereo_tracks: number; clips: number; markers: number;
  media_linked: number; media_written: number; missing_media: string[]; notes: string[];
  bus_levels?: string; buses_restored?: number; buses_created?: number; buses_matched?: number;
  inserts?: number; plugins?: number; automated_parameters?: number; soloed_tracks?: number;
};

/** OTIO import results: the same, with the timeline's name. */
export type OtioReport = Omit<AafReport, "composition"> & { timeline: string; stereo_pairs?: number };

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** What an AAF (or OTIO) import brought in, and what it couldn't. */
export function aafSummary(r: AafReport, mediaDir = "Audio/Imported/AAF"): string[] {
  const lines = [
    `"${r.composition}" at ${r.start_tc}: ${plural(r.tracks.length, "track")}${r.stereo_tracks ? ` (${r.stereo_tracks} stereo)` : ""}, ${plural(r.clips, "clip")}, ${plural(r.markers, "marker")}`,
    `Media: ${r.media_linked} linked in place, ${r.media_written} written to ${mediaDir}`,
  ];
  if (r.soloed_tracks) lines.push(`Solo: ${plural(r.soloed_tracks, "track")} soloed, as exported`);
  if (r.inserts) {
    const auto = r.automated_parameters ? `, ${plural(r.automated_parameters, "automated parameter")}` : "";
    lines.push(`Inserts: ${r.inserts} (${plural(r.plugins ?? 0, "plugin")}${auto})`);
  }
  if (r.bus_levels === "buses" && (r.buses_restored || r.buses_matched)) {
    const parts = [];
    if (r.buses_restored) parts.push(`${r.buses_restored} restored with their automation${r.buses_created ? ` (${r.buses_created} created)` : ""}`);
    if (r.buses_matched) parts.push(`${r.buses_matched} already matching, left as they were`);
    lines.push(`Buses: ${parts.join("; ")}`);
  }
  if (r.missing_media.length) lines.push("", "Missing media (clips left out):", ...r.missing_media.map((m) => "  " + m));
  if (r.notes.length) lines.push("", "Notes:", ...r.notes.map((n) => "  " + n));
  return lines;
}

export function otioSummary(r: OtioReport): string[] {
  const lines = aafSummary({ ...r, composition: r.timeline }, "Audio/Imported/OTIO");
  if (r.stereo_pairs) lines.splice(2, 0, `Stereo: ${plural(r.stereo_pairs, "left/right pair")} merged into one track each`);
  return lines;
}
