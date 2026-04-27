#!/usr/bin/env node
// install.js - resolves the a2a native binary for the current platform.
//
// Resolution order:
//   1. A2A_BINARY_PATH env var (developer override / CI)
//   2. Platform-specific optional dependency (e.g. @rover/a2a-cli-linux-x64)
//   3. `a2a` on PATH (for users who installed the binary separately)
//
// Run as postinstall to verify the binary resolves, and exported for bin.js.

'use strict';

const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

// Maps Node platform-arch → optional dependency package name.
const PLATFORM_MAP = {
  'darwin-arm64': '@rover/a2a-cli-darwin-arm64',
  'darwin-x64':   '@rover/a2a-cli-darwin-x64',
  'linux-arm64':  '@rover/a2a-cli-linux-arm64',
  'linux-x64':    '@rover/a2a-cli-linux-x64',
  'win32-x64':    '@rover/a2a-cli-win32-x64',
};

function platformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function binaryName() {
  return os.platform() === 'win32' ? 'a2a.exe' : 'a2a';
}

function assertExistingFile(envName, binPath) {
  let stat;
  try {
    stat = fs.statSync(binPath);
  } catch {
    throw new Error(
      `${envName} is set to "${binPath}" but the file does not exist.`
    );
  }

  if (!stat.isFile()) {
    throw new Error(`${envName} is set to "${binPath}" but it is not a file.`);
  }
}

function resolveFromEnv() {
  const a2aOverride = process.env.A2A_BINARY_PATH;
  if (a2aOverride) {
    assertExistingFile('A2A_BINARY_PATH', a2aOverride);
    return a2aOverride;
  }

  return null;
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
 * Check if `a2a` is on PATH and return its absolute path.
 */
function resolveFromPath() {
  try {
    const which = os.platform() === 'win32' ? 'where' : 'which';
    const result = execFileSync(which, ['a2a'], {
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
 * Returns the absolute path to the a2a binary.
 * Throws if the binary cannot be found.
 */
function getBinaryPath() {
  // 1. Developer/CI override
  const envOverride = resolveFromEnv();
  if (envOverride) return envOverride;

  // 2. Platform-specific optional dependency (preferred: bundled binary)
  const fromDep = resolveFromOptionalDep();
  if (fromDep) return fromDep;

  // 3. PATH fallback
  const fromPath = resolveFromPath();
  if (fromPath) return fromPath;

  const platform = platformKey();
  const supported = Object.keys(PLATFORM_MAP).join(', ');
  throw new Error(
    `a2a binary not found for platform "${platform}".\n` +
    `Supported platforms: ${supported}\n\n` +
    `To fix:\n` +
    `  - Reinstall:  npm install @rover/a2a-cli\n` +
    `  - Or set:     A2A_BINARY_PATH=/path/to/a2a`
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
    process.stderr.write(`a2a ready: ${bin}\n`);
  } catch (err) {
    // Don't fail npm install — the binary may be placed by a later build step.
    process.stderr.write(`[a2a postinstall] warning: ${err.message}\n`);
    process.exit(0);
  }
}

module.exports = { getBinaryPath };
