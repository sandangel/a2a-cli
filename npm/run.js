#!/usr/bin/env node

"use strict";

const { spawnSync } = require("child_process");
const { getBinaryPath } = require("./install");

let binary;
try {
  binary = getBinaryPath();
} catch (err) {
  console.error(`Error: ${err.message}`);
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), {
  cwd: process.cwd(),
  stdio: "inherit",
});

if (result.error) {
  console.error(`Error running a2a: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);
