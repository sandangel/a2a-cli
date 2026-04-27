# Contributing to a2a-cli

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
cargo build -p a2a-cli                        # dev build
cargo build -p a2a-cli --release              # release build
cargo test  -p a2a-cli                        # run all tests
cargo clippy -p a2a-cli -- -D warnings        # lint
cargo fmt   -p a2a-cli                        # format
```

## Pre-commit hooks

The repo uses [pre-commit](https://pre-commit.com/) to enforce quality gates locally:

| Hook | What it checks |
|------|---------------|
| `check-yaml` | YAML syntax on all `.yml`/`.yaml` files |
| `fmt` | `cargo fmt --check` on `a2a-cli` and `a2a-compat` |
| `clippy` | `cargo clippy -D warnings` on `a2a-cli` and `a2a-compat` |

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
[`a2a-cli/src/validate.rs`](a2a-cli/src/validate.rs).

## Repository layout

```
a2a-cli/        main CLI source crate directory
a2a-compat/     A2A v0.3 backward-compatibility layer
a2a-rs/         A2A Rust SDK (read-only git submodule)
gws-cli/        shared modules: fs_util, output, credential_store, validate (read-only)
npm/            npm wrapper package (@rover/a2a-cli)
```

> **Note:** `a2a-rs/` and `gws-cli/` are read-only references. Do not modify them.

## Relationship to gws-cli

`a2a-cli` is inspired by and shares implementation patterns with [**gws**](https://github.com/googleworkspace/cli), the Google Workspace CLI.
Three modules are included directly from `gws-cli/` via `#[path]` attributes in `a2a-cli/src/lib.rs`:

| Module | Source | Purpose |
|--------|--------|---------|
| `fs_util` | `gws-cli/crates/google-workspace-cli/src/fs_util.rs` | Atomic file writes |
| `output` | `gws-cli/crates/google-workspace-cli/src/output.rs` | Terminal-safe output formatting |
| `credential_store` | `gws-cli/crates/google-workspace-cli/src/credential_store.rs` | AES-256-GCM token encryption, keyring integration |

When changing auth, token storage, or output logic, check the corresponding implementation in `gws-cli/` first — the patterns are intentionally shared between the two tools.
