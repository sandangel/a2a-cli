# Contributing to agc

Thank you for contributing. All submissions require review via GitHub pull requests.

## Prerequisites

- Rust 1.85+
- `cargo` (no Makefile — use cargo directly)
- `pre-commit` — install once via `uv`:

```bash
uv tool install pre-commit
pre-commit install
```

## Build & test

```bash
cargo build -p agc                        # dev build
cargo build -p agc --release              # release build
cargo test  -p agc                        # run all tests
cargo clippy -p agc -- -D warnings        # lint
cargo fmt   -p agc                        # format
```

## Pre-commit hooks

The repo uses [pre-commit](https://pre-commit.com/) to enforce quality gates locally:

| Hook | What it checks |
|------|---------------|
| `check-yaml` | YAML syntax on all `.yml`/`.yaml` files |
| `fmt` | `cargo fmt --check` on `agc` and `a2a-compat` |
| `clippy` | `cargo clippy -D warnings` on `agc` and `a2a-compat` |

Hooks run automatically on `git commit`. To run them manually:

```bash
pre-commit run --all-files
```

## Submitting changes

1. **Branch** from `main`.
2. **Write conventional commits** — the release workflow uses them to detect the next version and generate changelogs:

   | Prefix | Effect |
   |--------|--------|
   | `feat:` | Minor version bump |
   | `fix:`, `perf:` | Patch bump |
   | `feat!:` / `BREAKING CHANGE` | Major bump |
   | `chore:`, `docs:`, `ci:` | No release — skipped in changelog |

3. **Open a PR** against `main`. All checks must pass:
   - `fmt / clippy / test` CI job
   - YAML validation
   - Rustfmt check

4. **Code review** — at least one approval from `@sg-genai/wovey-genai-repo-admins` is required (enforced by CODEOWNERS).

## Releasing

Releases are cut manually from the Actions tab:

1. Go to **Actions → Release → Run workflow**.
2. Choose `rc` to cut a pre-release, or `release` to promote the latest RC to a stable release.
   - `rc`: tags HEAD of `main` as `v{next}-rc.{N}`, publishes a pre-release on GitHub.
   - `release`: retags the latest RC commit as `v{next}` (no new code), publishes a stable release and pushes to npm.

The next version is determined automatically from conventional commits since the last tag.

## Input validation

This CLI is designed to be invoked by AI coding tools, so all user-supplied
inputs must be treated as potentially adversarial. Before adding any flag that
accepts a path, URL, or resource identifier, read the validation rules in
[`AGENTS.md`](AGENTS.md) and use the appropriate helper from
[`agc/src/validate.rs`](agc/src/validate.rs).

## Repository layout

```
agc/            main CLI binary crate
a2a-compat/     A2A v0.3 backward-compatibility layer
a2a-rs/         A2A Rust SDK (read-only git submodule)
gws-cli/        shared modules: fs_util, output, credential_store, validate (read-only)
npm/            npm wrapper package (@rover/agent-cli)
```

> **Note:** `a2a-rs/` and `gws-cli/` are read-only references. Do not modify them.
