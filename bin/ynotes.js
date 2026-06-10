#!/usr/bin/env node
'use strict';

// Launcher for the ynotes CLI installed from npm.
//
// The native binary is NOT in this package. It lives in a per-platform
// package (`ynotes-darwin-arm64`, `ynotes-linux-x64`, ...) declared as an
// optionalDependency of `ynotes`. npm installs only the one whose `os`, `cpu`,
// and `libc` match the host, so an install pulls a single binary rather than
// all of them. This shim's whole job is to find that binary and hand off to it
// transparently.

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

// On Linux, a glibc binary will not run on musl (Alpine) and vice versa.
// npm's `libc` field lets a modern client install only the matching package,
// but older clients ignore it — so the launcher detects musl itself and uses
// it only to *order* the candidates; resolution below falls back to whichever
// variant is actually present, so a wrong guess still works.
function linuxIsMusl() {
  const loader =
    process.arch === 'arm64' ? '/lib/ld-musl-aarch64.so.1' : '/lib/ld-musl-x86_64.so.1';
  try {
    return fs.existsSync(loader);
  } catch {
    return false;
  }
}

// Host -> per-platform package(s) to try, best match first.
function candidatePackages() {
  const arch = { x64: 'x64', arm64: 'arm64' }[process.arch];
  if (!arch) return [];
  switch (process.platform) {
    case 'darwin':
      return [`ynotes-darwin-${arch}`];
    case 'win32':
      return [`ynotes-win32-${arch}`];
    case 'linux':
      return linuxIsMusl()
        ? [`ynotes-linux-musl-${arch}`, `ynotes-linux-${arch}`]
        : [`ynotes-linux-${arch}`, `ynotes-linux-musl-${arch}`];
    default:
      return [];
  }
}

function resolveBinary() {
  const candidates = candidatePackages();
  if (candidates.length === 0) {
    return {
      error: `unsupported platform ${process.platform} ${process.arch} — see the README to build from source`,
    };
  }
  const exe = process.platform === 'win32' ? 'ynotes.exe' : 'ynotes';
  for (const pkg of candidates) {
    try {
      // Resolve via the package's manifest: a path that always exists if the
      // package is installed, unlike the binary (which lacks an extension on
      // Unix and so is not a Node-resolvable module specifier).
      const manifest = require.resolve(`${pkg}/package.json`);
      return { binary: path.join(path.dirname(manifest), 'bin', exe) };
    } catch {
      // Not installed — try the next candidate.
    }
  }
  return {
    error:
      `no ynotes binary package is installed (tried: ${candidates.join(', ')}).\n` +
      `Reinstall ynotes without --no-optional, or download a prebuilt binary ` +
      `from the GitHub Releases page.`,
  };
}

const resolved = resolveBinary();
if (resolved.error) {
  console.error(`ynotes: ${resolved.error}`);
  process.exit(1);
}

// stdio: 'inherit' so the child owns the real terminal — colour detection,
// stdin piping, and exit codes all behave as if the binary were run directly.
const result = spawnSync(resolved.binary, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
  console.error(`ynotes: failed to launch binary: ${result.error.message}`);
  process.exit(1);
}
// A signal-terminated child has a null status; surface that as a failure.
process.exit(result.status === null ? 1 : result.status);
