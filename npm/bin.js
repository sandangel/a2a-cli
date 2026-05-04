#!/usr/bin/env node
// bin.js — self-replacing shim (macOS/Linux only).
//
// On first run, copies the native a2a binary over this file and re-execs it
// so all future invocations bypass Node entirely.
//
// Windows: self-replace is skipped because Windows locks running files and
// npm always invokes the bin through a .cmd wrapper anyway. Falls back to
// spawning the native binary directly on every call.
//
// If the copy fails for any reason (read-only fs, --ignore-scripts, etc.)
// it falls back to spawning — works correctly, just keeps Node in the loop.

'use strict';

const { spawnSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const { getBinaryPath } = require('./install');

let binary;
try {
  binary = getBinaryPath();
} catch (err) {
  process.stderr.write(`Error: ${err.message}\n`);
  process.exit(1);
}

// Self-replace: macOS and Linux only.
if (os.platform() !== 'win32') {
  const self = __filename;
  try {
    fs.copyFileSync(binary, self);
    fs.chmodSync(self, 0o755);
    // Re-exec the now-native binary in place of this process.
    const result = spawnSync(self, process.argv.slice(2), {
      stdio: 'inherit',
      env: process.env,
    });
    process.exit(result.status ?? 0);
  } catch {
    // Self-replace failed — fall through to spawn below.
  }
}

const result = spawnSync(binary, process.argv.slice(2), {
  stdio: 'inherit',
  env: process.env,
});

if (result.error) {
  process.stderr.write(`Failed to run a2a: ${result.error.message}\n`);
  process.exit(1);
}

process.exit(result.status ?? 0);
