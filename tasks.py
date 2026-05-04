"""
Local development tasks — run with: uv run inv <task>

  uv run inv build        # dev build (current host)
  uv run inv test         # run all tests
  uv run inv lint         # fmt check + clippy
  uv run inv fix          # fmt + clippy --fix
  uv run inv sync-npm-version --version=1.2.3
  uv run inv sync-cargo-version --version=1.2.3
  uv run inv package-crates --version=1.2.3
  uv run inv update-skills
  uv run inv install      # build release + copy to ~/.local/bin/a2a
  uv run inv clean        # cargo clean
  uv run inv -l           # list all tasks
"""

import json
import os
import re
import hashlib
import shutil
import tarfile
import tempfile
from pathlib import Path

from invoke import task

ROOT = Path(__file__).resolve().parent
PKG = "a2a-protocol-cli"
COMPAT_PKG = "a2a-protocol-compat"
BIN = "target/debug/a2a"
RELEASE_BIN = "target/release/a2a"
NPM_PACKAGE_JSON = [
    Path("npm/package.json"),
    Path("npm/packages/a2a-protocol-cli-darwin-arm64/package.json"),
    Path("npm/packages/a2a-protocol-cli-darwin-x64/package.json"),
    Path("npm/packages/a2a-protocol-cli-linux-arm64/package.json"),
    Path("npm/packages/a2a-protocol-cli-linux-x64/package.json"),
    Path("npm/packages/a2a-protocol-cli-win32-x64/package.json"),
]
CRATE_ARTIFACT_DIR = ROOT / "target" / "package-artifacts"


def _replace_once(path, pattern, replacement, label):
    text = path.read_text()
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"could not update {label} in {path}")
    path.write_text(updated)


def _sync_cargo_version_at(root, version):
    compat_toml = root / "a2a-compat" / "Cargo.toml"
    cli_toml = root / "a2a-cli" / "Cargo.toml"

    print(f"stamping Cargo packages with {version}")

    _replace_once(
        compat_toml,
        r'(\[package\]\s+name = "a2a-protocol-compat"\s+version = ")[^"]+(")',
        lambda match: f"{match.group(1)}{version}{match.group(2)}",
        "a2a-protocol-compat package version",
    )
    _replace_once(
        cli_toml,
        r'(\[package\]\s+name = "a2a-protocol-cli"\s+version = ")[^"]+(")',
        lambda match: f"{match.group(1)}{version}{match.group(2)}",
        "a2a-protocol-cli package version",
    )
    _replace_once(
        cli_toml,
        (
            r'(a2a-compat = \{ package = "a2a-protocol-compat", version = ")'
            r'[^"]+(", path = "\.\./a2a-compat" \})'
        ),
        lambda match: f"{match.group(1)}{version}{match.group(2)}",
        "a2a-protocol-compat dependency version",
    )

    print("  stamped a2a-protocol-compat")
    print("  stamped a2a-protocol-cli")


def _publish_ignore(path, names):
    current = Path(path).resolve()
    ignored = set()

    if current == ROOT:
        ignored.update({".git", ".venv", "target"}.intersection(names))
    elif current == ROOT / "a2a-cli":
        ignored.update({"skills"}.intersection(names))

    return ignored


def _vendor_publish_modules(stage_repo):
    vendored_modules = [
        (
            ROOT / "gws-cli/crates/google-workspace-cli/src/fs_util.rs",
            stage_repo / "a2a-cli/src/fs_util.rs",
        ),
        (
            ROOT / "gws-cli/crates/google-workspace-cli/src/formatter.rs",
            stage_repo / "a2a-cli/src/formatter.rs",
        ),
    ]
    for source, destination in vendored_modules:
        shutil.copy2(source, destination)

    lib_rs = stage_repo / "a2a-cli/src/lib.rs"
    text = lib_rs.read_text()
    replacements = [
        (
            """#[rustfmt::skip]
#[allow(clippy::collapsible_if)]
#[path = "../../gws-cli/crates/google-workspace-cli/src/fs_util.rs"]
pub mod fs_util;""",
            """#[allow(clippy::collapsible_if)]
pub mod fs_util;""",
        ),
        (
            """#[rustfmt::skip]
#[allow(clippy::should_implement_trait, clippy::collapsible_if)]
#[path = "../../gws-cli/crates/google-workspace-cli/src/formatter.rs"]
pub mod formatter;""",
            """#[allow(clippy::should_implement_trait, clippy::collapsible_if)]
pub mod formatter;""",
        ),
    ]

    for old, new in replacements:
        if old not in text:
            raise RuntimeError(f"could not rewrite vendored module declaration in {lib_rs}")
        text = text.replace(old, new, 1)

    lib_rs.write_text(text)


def _mark_skill_internal(path):
    text = path.read_text()
    if re.search(r"(?m)^\s*internal:\s*true\s*$", text):
        return False
    if not text.startswith("---\n"):
        return False

    frontmatter, sep, body = text[len("---\n") :].partition("---\n")
    if not sep:
        return False

    lines = frontmatter.splitlines(keepends=True)
    for index, line in enumerate(lines):
        if line == "metadata:\n":
            lines.insert(index + 1, "  internal: true\n")
            path.write_text("---\n" + "".join(lines) + sep + body)
            return True

    if frontmatter and not frontmatter.endswith("\n"):
        frontmatter += "\n"
    frontmatter += "metadata:\n  internal: true\n"
    path.write_text("---\n" + frontmatter + sep + body)
    return True


def _skills_command():
    if shutil.which("bunx"):
        return "bunx skills"
    return "npx skills"


def _iter_files(root, includes):
    for include in includes:
        path = root / include
        if path.is_file():
            yield path
        elif path.is_dir():
            yield from sorted(item for item in path.rglob("*") if item.is_file())


def _write_crate_archive(package_dir, package_name, version, includes):
    archive = CRATE_ARTIFACT_DIR / f"{package_name}-{version}.crate"
    prefix = f"{package_name}-{version}"

    with tarfile.open(archive, "w:gz") as tar:
        for source in _iter_files(package_dir, includes):
            tar.add(source, arcname=f"{prefix}/{source.relative_to(package_dir)}")

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_suffix(archive.suffix + ".sha256")
    checksum.write_text(f"{digest}  {archive.name}\n")
    print(f"packaged {archive}")
    print(f"wrote {checksum}")


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
    c.run(f"cargo fmt -p '{PKG}' -p '{COMPAT_PKG}' -- --check", pty=True)
    c.run(f"cargo clippy -p '{PKG}' -p '{COMPAT_PKG}' -- -D warnings", pty=True)


@task
def fix(c):
    """Auto-fix formatting and clippy lints."""
    c.run(f"cargo fmt -p '{PKG}' -p '{COMPAT_PKG}'", pty=True)
    c.run(f"cargo clippy -p '{PKG}' -p '{COMPAT_PKG}' --fix --allow-dirty -- -D warnings", pty=True)


@task(pre=[build])
def install(c, dest="~/.local/bin"):
    """Build (dev) and install the binary to dest (default: ~/.local/bin)."""
    dest = os.path.expanduser(dest)
    os.makedirs(dest, exist_ok=True)
    c.run(f"cp {BIN} {dest}/a2a")
    print(f"installed → {dest}/a2a")


@task(pre=[release])
def install_release(c, dest="~/.local/bin"):
    """Build (release) and install the binary to dest (default: ~/.local/bin)."""
    dest = os.path.expanduser(dest)
    os.makedirs(dest, exist_ok=True)
    c.run(f"cp {RELEASE_BIN} {dest}/a2a")
    print(f"installed → {dest}/a2a")


@task
def clean(c):
    """Remove build artifacts."""
    c.run("cargo clean", pty=True)


@task
def sync_cargo_version(c, version):
    """Stamp VERSION into Rust packages."""
    _sync_cargo_version_at(ROOT, version)


@task
def sync_npm_version(c, version):
    """Stamp VERSION into all npm package.json files."""
    print(f"stamping npm packages with {version}")

    for package_path in NPM_PACKAGE_JSON:
        path = ROOT / package_path
        package = json.loads(path.read_text())
        package["version"] = version

        optional_dependencies = package.get("optionalDependencies")
        if optional_dependencies:
            for dependency in optional_dependencies:
                optional_dependencies[dependency] = version

        path.write_text(json.dumps(package, indent=2) + "\n")
        print(f"  stamped {package_path}")


@task
def update_skills(c):
    """Regenerate public skills and update repo-local agent skills."""
    print("Regenerating skills/a2a/SKILL.md...")
    c.run(f"cargo run -p '{PKG}' -- generate-skills", pty=True)

    skills_cmd = _skills_command()
    print(f"Updating .agents/skills/ with {skills_cmd}...")
    c.run(f"{skills_cmd} update --yes", pty=True)

    print("Marking .agents/skills/ as internal...")
    updated = 0
    for path in sorted((ROOT / ".agents" / "skills").glob("*/SKILL.md")):
        if _mark_skill_internal(path):
            print(f"  marked: {path.relative_to(ROOT)}")
            updated += 1

    print(f"Done. {updated} file(s) updated.")


@task
def package_crates(c, version, dry_run=False):
    """Package Rust crate source archives into target/package-artifacts."""
    with tempfile.TemporaryDirectory() as stage:
        stage_repo = Path(stage) / "repo"
        shutil.copytree(ROOT, stage_repo, ignore=_publish_ignore)
        _vendor_publish_modules(stage_repo)
        _sync_cargo_version_at(stage_repo, version)

        with c.cd(str(stage_repo)):
            c.run(f"cargo check -p {PKG} -p {COMPAT_PKG}", pty=True)
            if dry_run:
                c.run(f"cargo package -p {COMPAT_PKG} --allow-dirty --list >/dev/null", pty=True)
                c.run(f"cargo package -p {PKG} --allow-dirty --list >/dev/null", pty=True)
                return

        CRATE_ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)
        _write_crate_archive(
            stage_repo / "a2a-compat",
            COMPAT_PKG,
            version,
            ["Cargo.toml", "src"],
        )
        _write_crate_archive(
            stage_repo / "a2a-cli",
            PKG,
            version,
            ["Cargo.toml", "Cargo.lock", "README.md", "build.rs", "proto", "src", "tests"],
        )


@task
def publish_crates(c, version, dry_run=False):
    """Compatibility alias for packaging Rust crates as GitHub artifacts."""
    package_crates(c, version=version, dry_run=dry_run)


@task
def version(c):
    """Print the version the binary would report."""
    c.run(
        "git describe --tags --match 'v[0-9]*' --always | sed 's/^v//'",
        pty=True,
    )
