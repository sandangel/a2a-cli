#!/usr/bin/env node
// bin.js — thin shim that spawns the native agc binary.
// Passes all arguments through and inherits stdio so interactive
// prompts (OAuth device-code flow, etc.) work correctly.

'use strict';

const { spawnSync } = require('child_process');
const { getBinaryPath } = require('./install');

let binary;
try {
  binary = getBinaryPath();
} catch (err) {
  process.stderr.write(`Error: ${err.message}\n`);
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), {
  stdio: 'inherit',
  env: process.env,
});

if (result.error) {
  process.stderr.write(`Failed to run agc: ${result.error.message}\n`);
  process.exit(1);
}

process.exit(result.status ?? 0);
