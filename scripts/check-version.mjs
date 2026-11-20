#!/usr/bin/env node
// Fails a release when the tag (e.g. v0.2.0) doesn't match the versions in the repo.
import { readVersions } from "./bump-version.mjs";

const tag = (process.argv[2] ?? "").replace(/^refs\/tags\//, "");
const want = tag.replace(/^v/, "");
const have = readVersions();
const bad = Object.entries(have).filter(([, v]) => v !== want);
if (!want || bad.length) {
  console.error(`tag ${tag || "(none)"} does not match versions ${JSON.stringify(have)} — run: node scripts/bump-version.mjs ${want || "<x.y.z>"}`);
  process.exit(1);
}
console.log(`version ${want} ✓`);
