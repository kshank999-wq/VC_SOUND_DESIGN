import { test } from "node:test";
import assert from "node:assert/strict";
import { allFolders, folderTree, parentOf } from "../src/folders.ts";

const tracks = [
  { id: "bg", folder: "Scene 12/Backgrounds & Atmos" },
  { id: "foley1", folder: "Scene 12/Foley" },
  { id: "loose", folder: "" },
  { id: "foley2", folder: "Scene 12/Foley" },
  { id: "s13", folder: " Scene 13 / Foley " },
];

const shape = (items: ReturnType<typeof folderTree>) =>
  items.map((i) => (i.kind === "folder" ? `${"  ".repeat(i.depth)}[${i.name}] ${i.trackIds.length}` : `${"  ".repeat(i.depth)}${i.track.id}`));

test("nests by path and keeps first-appearance order", () => {
  assert.deepEqual(shape(folderTree(tracks, new Set())), [
    "[Scene 12] 3",
    "  [Backgrounds & Atmos] 1",
    "    bg",
    "  [Foley] 2",
    "    foley1",
    "    foley2",
    "loose",
    "[Scene 13] 1",
    "  [Foley] 1",
    "    s13",
  ]);
});

test("collapsed folders hide their contents but report them", () => {
  const items = folderTree(tracks, new Set(["Scene 12"]));
  assert.deepEqual(shape(items), ["[Scene 12] 3", "loose", "[Scene 13] 1", "  [Foley] 1", "    s13"]);
  const f = items[0];
  assert.ok(f.kind === "folder" && f.collapsed && f.trackIds.join() === "bg,foley1,foley2");
});

test("folder helpers", () => {
  assert.deepEqual(allFolders(tracks), ["Scene 12", "Scene 12/Backgrounds & Atmos", "Scene 12/Foley", "Scene 13", "Scene 13/Foley"]);
  assert.equal(parentOf("Scene 12/Foley"), "Scene 12");
  assert.equal(parentOf("Scene 12"), "");
});
