#!/usr/bin/env node
// Propagate the crate version into every npm manifest.
//
// `Cargo.toml` is the single source of truth for the version (AGENTS.md
// invariant 9). `package.json` and `npm/*/package.json` are derived from it:
// run this script to rewrite them, or with `--check` to verify they match
// without writing — which is what the `version-sync` CI job does.
//
// Usage:
//   node scripts/sync-version.mjs            # rewrite the npm manifests
//   node scripts/sync-version.mjs --check    # exit 1 if any is out of sync

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const check = process.argv.includes('--check');

/** The `version` of the `[workspace.package]` table in Cargo.toml. */
function crateVersion() {
  const toml = readFileSync(join(root, 'Cargo.toml'), 'utf8');
  // Isolate the `[workspace.package]` table (up to the next `[table]` header)
  // so a `version = ` key from a dependency table can never be picked up.
  const table = toml.split(/^\[/m).find((s) => s.startsWith('workspace.package]'));
  const match = table?.match(/^version = "([^"]+)"/m);
  if (!match) throw new Error('no [workspace.package] version found in Cargo.toml');
  return match[1];
}

const version = crateVersion();
const platforms = [
  'darwin-arm64',
  'darwin-x64',
  'linux-arm64',
  'linux-musl-arm64',
  'linux-musl-x64',
  'linux-x64',
  'win32-x64',
];
const manifests = [
  { path: 'package.json', main: true },
  ...platforms.map((p) => ({ path: `npm/${p}/package.json`, main: false })),
];

let drift = false;
for (const { path, main } of manifests) {
  const file = join(root, path);
  const json = JSON.parse(readFileSync(file, 'utf8'));
  let changed = false;

  if (json.version !== version) {
    json.version = version;
    changed = true;
  }
  // The main package pins each per-platform optionalDependency to the exact
  // version, so `npm install ynotes` can only ever pull a matching binary.
  if (main && json.optionalDependencies) {
    for (const dep of Object.keys(json.optionalDependencies)) {
      if (json.optionalDependencies[dep] !== version) {
        json.optionalDependencies[dep] = version;
        changed = true;
      }
    }
  }

  if (!changed) continue;
  drift = true;
  if (check) {
    console.error(`out of sync: ${path}`);
  } else {
    writeFileSync(file, `${JSON.stringify(json, null, 2)}\n`);
    console.log(`synced ${path} -> ${version}`);
  }
}

if (check && drift) {
  console.error('\nnpm manifests are stale. Run: node scripts/sync-version.mjs');
  process.exit(1);
}
console.log(check ? `all npm manifests match Cargo.toml (${version})` : `done (${version})`);
