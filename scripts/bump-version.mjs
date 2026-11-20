#!/usr/bin/env node
// Set the app version everywhere it lives: `node scripts/bump-version.mjs 0.2.0`
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
export const FILES = {
  cargo: join(root, "Cargo.toml"),
  tauri: join(root, "crates/vcsd-app/tauri.conf.json"),
  ui: join(root, "ui/package.json"),
};

export function readVersions() {
  const cargo = /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/s.exec(readFileSync(FILES.cargo, "utf8"))?.[1];
  const tauri = JSON.parse(readFileSync(FILES.tauri, "utf8")).version;
  const ui = JSON.parse(readFileSync(FILES.ui, "utf8")).version;
  return { cargo, tauri, ui };
}

export function bump(version) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version)) throw new Error(`not a semver version: ${version}`);
  const c = readFileSync(FILES.cargo, "utf8").replace(/(\[workspace\.package\][^[]*?\nversion = ")[^"]+(")/s, `$1${version}$2`);
  writeFileSync(FILES.cargo, c);
  // edit only the top-level "version" string so file formatting is preserved
  for (const f of [FILES.tauri, FILES.ui]) {
    const text = readFileSync(f, "utf8");
    const out = text.replace(/^(\s{2}"version":\s*")[^"]*(")/m, `$1${version}$2`);
    if (out === text && JSON.parse(text).version !== version) throw new Error(`no top-level version in ${f}`);
    writeFileSync(f, out);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  bump(process.argv[2] ?? "");
  console.log(readVersions());
}
