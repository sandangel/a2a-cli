#!/usr/bin/env node
// bin.js — thin shim that spawns the native rover binary.
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
  // Pass environment through so A2A_AGENT_URL, A2A_BEARER_TOKEN, etc. work.
  env: process.env,
});

if (result.error) {
  process.stderr.write(`Failed to run a2a: ${result.error.message}\n`);
  process.exit(1);
}

process.exit(result.status ?? 0);
