#!/usr/bin/env node
// install.js — resolves the a2a native binary for the current platform.
//
// Resolution order:
//   1. AGC_BINARY_PATH env var (developer override / CI)
//   2. Platform-specific optional dependency (e.g. @woven/a2a-linux-x64)
//   3. `a2a` on PATH (for users who installed the Go binary separately)
//
// Called automatically as a postinstall script and exported for use by bin.js.

'use strict';

const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

// Map node platform/arch → package name suffix
const PLATFORM_MAP = {
  'linux-x64':    '@rover/agent-cli-linux-x64',
  'linux-arm64':  '@rover/agent-cli-linux-arm64',
  'darwin-x64':   '@rover/agent-cli-darwin-x64',
  'darwin-arm64': '@rover/agent-cli-darwin-arm64',
  'win32-x64':    '@rover/agent-cli-win32-x64',
};

function platformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function binaryName() {
  return os.platform() === 'win32' ? 'agc.exe' : 'agc';
}

// Resolve binary from the installed platform-specific optional dependency.
function resolveFromOptionalDep() {
  const pkgName = PLATFORM_MAP[platformKey()];
  if (!pkgName) return null;

  try {
    // resolve() finds the package root; the binary is at bin/<name>
    const pkgDir = path.dirname(require.resolve(`${pkgName}/package.json`));
    const bin = path.join(pkgDir, 'bin', binaryName());
    if (fs.existsSync(bin)) return bin;
  } catch {
    // optional dep not installed — normal when npm skipped it
  }
  return null;
}

// Check if `a2a` is on PATH and return the full path if found.
function resolveFromPath() {
  try {
    const which = os.platform() === 'win32' ? 'where' : 'which';
    const result = execFileSync(which, ['agc'], { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
    const bin = result.trim().split('\n')[0];
    if (bin && fs.existsSync(bin)) return bin;
  } catch {
    // not on PATH
  }
  return null;
}

/**
 * Returns the absolute path to the rover binary.
 * Throws if the binary cannot be found.
 */
function getBinaryPath() {
  // 1. Developer/CI override
  const envOverride = process.env.AGC_BINARY_PATH;
  if (envOverride) {
    if (!fs.existsSync(envOverride)) {
      throw new Error(`AGC_BINARY_PATH is set to "${envOverride}" but file does not exist.`);
    }
    return envOverride;
  }

  // 2. Optional platform dependency
  const fromDep = resolveFromOptionalDep();
  if (fromDep) return fromDep;

  // 3. PATH fallback
  const fromPath = resolveFromPath();
  if (fromPath) return fromPath;

  const platform = platformKey();
  const supported = Object.keys(PLATFORM_MAP).join(', ');
  throw new Error(
    `agc binary not found for platform "${platform}".\n` +
    `Supported platforms: ${supported}\n\n` +
    `To fix:\n` +
    `  • Reinstall:  npm install @rover/agent-cli\n` +
    `  • Or build:   cd a2a-cli && make install\n` +
    `  • Or set:     AGC_BINARY_PATH=/path/to/agc`
  );
}

// Run as postinstall: verify the binary resolves correctly.
if (require.main === module) {
  try {
    const bin = getBinaryPath();
    // Ensure the binary is executable on Unix.
    if (os.platform() !== 'win32') {
      try { fs.chmodSync(bin, 0o755); } catch {}
    }
    // Quick sanity check.
    execFileSync(bin, ['--version'], { stdio: 'pipe' });
    console.log(`a2a CLI ready: ${bin}`);
  } catch (err) {
    // postinstall failures should not block npm install.
    // The binary may be placed later by a build step.
    process.stderr.write(`[a2a postinstall] Warning: ${err.message}\n`);
    process.exit(0);
  }
}

module.exports = { getBinaryPath };
