#!/usr/bin/env node
// install.js — resolves the agc native binary for the current platform.
//
// Resolution order:
//   1. AGC_BINARY_PATH env var (developer override / CI)
//   2. Platform-specific optional dependency (e.g. @rover/agent-cli-linux-x64)
//   3. `agc` on PATH (for users who installed the binary separately)
//
// Run as postinstall to verify the binary resolves, and exported for bin.js.

'use strict';

const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

// Maps Node platform-arch → optional dependency package name.
const PLATFORM_MAP = {
  'darwin-arm64': '@rover/agent-cli-darwin-arm64',
  'darwin-x64':   '@rover/agent-cli-darwin-x64',
  'linux-arm64':  '@rover/agent-cli-linux-arm64',
  'linux-x64':    '@rover/agent-cli-linux-x64',
  'win32-x64':    '@rover/agent-cli-win32-x64',
};

function platformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function binaryName() {
  return os.platform() === 'win32' ? 'agc.exe' : 'agc';
}

/**
 * Resolve binary from the installed platform-specific optional dependency.
 * Returns the absolute path, or null if the dep is not installed.
 */
function resolveFromOptionalDep() {
  const pkgName = PLATFORM_MAP[platformKey()];
  if (!pkgName) return null;
  try {
    const pkgDir = path.dirname(require.resolve(`${pkgName}/package.json`));
    const bin = path.join(pkgDir, 'bin', binaryName());
    if (fs.existsSync(bin)) return bin;
  } catch {
    // optional dep not installed — normal when npm skipped it for this platform
  }
  return null;
}

/**
 * Check if `agc` is on PATH and return its absolute path.
 */
function resolveFromPath() {
  try {
    const which = os.platform() === 'win32' ? 'where' : 'which';
    const result = execFileSync(which, ['agc'], {
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    const bin = result.trim().split('\n')[0];
    if (bin && fs.existsSync(bin)) return bin;
  } catch {
    // not on PATH
  }
  return null;
}

/**
 * Returns the absolute path to the agc binary.
 * Throws if the binary cannot be found.
 */
function getBinaryPath() {
  // 1. Developer/CI override
  const envOverride = process.env.AGC_BINARY_PATH;
  if (envOverride) {
    if (!fs.existsSync(envOverride)) {
      throw new Error(
        `AGC_BINARY_PATH is set to "${envOverride}" but the file does not exist.`
      );
    }
    return envOverride;
  }

  // 2. Platform-specific optional dependency (preferred: bundled binary)
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
    `  • Or set:     AGC_BINARY_PATH=/path/to/agc`
  );
}

// When run as postinstall: verify the binary resolves and is executable.
if (require.main === module) {
  try {
    const bin = getBinaryPath();
    if (os.platform() !== 'win32') {
      try { fs.chmodSync(bin, 0o755); } catch {}
    }
    execFileSync(bin, ['--version'], { stdio: 'pipe' });
    process.stderr.write(`agc ready: ${bin}\n`);
  } catch (err) {
    // Don't fail npm install — the binary may be placed by a later build step.
    process.stderr.write(`[agc postinstall] warning: ${err.message}\n`);
    process.exit(0);
  }
}

module.exports = { getBinaryPath };
