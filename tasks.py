"""
Local development tasks — run with: uv run inv <task>

  uv run inv build        # dev build (current host)
  uv run inv test         # run all tests
  uv run inv lint         # fmt check + clippy
  uv run inv fix          # fmt + clippy --fix
  uv run inv install      # build release + copy to ~/.local/bin/a2a
  uv run inv clean        # cargo clean
  uv run inv -l           # list all tasks
"""

from invoke import task

PKG = "a2a-cli@0.0.0"
BIN = "target/debug/a2a"
RELEASE_BIN = "target/release/a2a"


@task
def build(c):
    """Dev build for the current host."""
    c.run(f"cargo build -p '{PKG}'", pty=True)


@task
def release(c):
    """Release build for the current host."""
    c.run(f"cargo build -p '{PKG}' --release", pty=True)


@task
def test(c, filter=""):
    """Run all tests (pass filter= to narrow by name)."""
    cmd = f"cargo test -p '{PKG}'"
    if filter:
        cmd += f" {filter}"
    c.run(cmd, pty=True)


@task
def lint(c):
    """Check formatting and run clippy."""
    c.run(f"cargo fmt -p a2a-cli -- --check", pty=True)
    c.run(f"cargo clippy -p '{PKG}' -p a2a-compat -- -D warnings", pty=True)


@task
def fix(c):
    """Auto-fix formatting and clippy lints."""
    c.run("cargo fmt -p a2a-cli", pty=True)
    c.run(f"cargo clippy -p '{PKG}' -p a2a-compat --fix --allow-dirty -- -D warnings", pty=True)


@task(pre=[build])
def install(c, dest="~/.local/bin"):
    """Build (dev) and install the binary to dest (default: ~/.local/bin)."""
    import os
    dest = os.path.expanduser(dest)
    os.makedirs(dest, exist_ok=True)
    c.run(f"cp {BIN} {dest}/a2a")
    print(f"installed → {dest}/a2a")


@task(pre=[release])
def install_release(c, dest="~/.local/bin"):
    """Build (release) and install the binary to dest (default: ~/.local/bin)."""
    import os
    dest = os.path.expanduser(dest)
    os.makedirs(dest, exist_ok=True)
    c.run(f"cp {RELEASE_BIN} {dest}/a2a")
    print(f"installed → {dest}/a2a")


@task
def clean(c):
    """Remove build artifacts."""
    c.run("cargo clean", pty=True)


@task
def version(c):
    """Print the version the binary would report."""
    c.run(
        "git describe --tags --match 'v[0-9]*' --always | sed 's/^v//'",
        pty=True,
    )
