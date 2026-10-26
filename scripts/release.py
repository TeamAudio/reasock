#!/usr/bin/env python3
"""Release helper, after XY Pad's.

Changes are described as they land, in releases/pending/<anything>.<level>.md
where <level> is patch, minor, or major. `prepare` picks the largest level,
bumps the version in Cargo.toml and Cargo.lock, compiles the notes into
releases/changes-<version>.md, and removes the pending files.

    python3 scripts/release.py prepare [--version X.Y.Z] [--dry-run]
    python3 scripts/release.py version
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
RELEASES = ROOT / "releases"
PENDING = RELEASES / "pending"
LEVELS = ("patch", "minor", "major")
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")


class ReleaseError(RuntimeError):
    pass


def current_version() -> str:
    match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', CARGO_TOML.read_text(encoding="utf-8"))
    if not match:
        raise ReleaseError("no version in Cargo.toml")
    return match.group(1)


def bump(version: str, level: str) -> str:
    match = SEMVER.match(version)
    if not match:
        raise ReleaseError(f"not a version: {version!r}")
    major, minor, patch = (int(part) for part in match.groups())
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def pending_notes() -> list[tuple[str, Path]]:
    notes = []
    for path in sorted(PENDING.glob("*.md")):
        level = path.name.rsplit(".", 2)[-2].lower() if path.name.count(".") >= 2 else ""
        if level not in LEVELS:
            raise ReleaseError(f"{path.name}: name it <anything>.patch.md, .minor.md, or .major.md")
        notes.append((level, path))
    return notes


def set_version(version: str) -> None:
    """Updates the package version in Cargo.toml and its entry in Cargo.lock."""
    toml = CARGO_TOML.read_text(encoding="utf-8")
    toml, count = re.subn(r'(?m)^(version\s*=\s*")[^"]+(")', rf"\g<1>{version}\g<2>", toml, count=1)
    if count != 1:
        raise ReleaseError("couldn't update the version in Cargo.toml")
    lock = CARGO_LOCK.read_text(encoding="utf-8")
    lock, count = re.subn(
        r'(\[\[package\]\]\nname = "reasock"\nversion = ")[^"]+(")', rf"\g<1>{version}\g<2>", lock
    )
    if count != 1:
        raise ReleaseError("couldn't update reasock's version in Cargo.lock")
    CARGO_TOML.write_text(toml, encoding="utf-8")
    CARGO_LOCK.write_text(lock, encoding="utf-8")


def compile_notes(notes: list[tuple[str, Path]]) -> str:
    lines = []
    for _, path in notes:
        for line in path.read_text(encoding="utf-8").strip().splitlines():
            line = line.strip()
            if line:
                lines.append(line if line.startswith(("-", "*")) else f"- {line}")
    return "\n".join(lines) + "\n"


def output(**values: str) -> None:
    """Prints values, and hands them to later workflow steps when in Actions."""
    for key, value in values.items():
        print(f"{key}={value}")
    if path := os.environ.get("GITHUB_OUTPUT"):
        with open(path, "a", encoding="utf-8") as file:
            for key, value in values.items():
                file.write(f"{key}={value}\n")


def cmd_prepare(args: argparse.Namespace) -> None:
    current = current_version()
    notes = pending_notes()
    if not notes:
        raise ReleaseError("no pending notes in releases/pending")
    level = max((level for level, _ in notes), key=LEVELS.index)
    version = args.version or bump(current, level)
    if not SEMVER.match(version):
        raise ReleaseError(f"not a version: {version!r}")
    changes = RELEASES / f"changes-{version}.md"
    if changes.exists():
        raise ReleaseError(f"{changes.relative_to(ROOT)} already exists")
    text = compile_notes(notes)
    if not args.dry_run:
        set_version(version)
        changes.write_text(text, encoding="utf-8")
        for _, path in notes:
            path.unlink()
    print(text, end="", file=sys.stderr)
    output(version=version, previous=current, bump_level=level, changes_file=str(changes.relative_to(ROOT)))


def cmd_version(_: argparse.Namespace) -> None:
    output(version=current_version())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("prepare", help="bump the version and compile pending notes")
    prepare.add_argument("--version", help="use this version instead of bumping")
    prepare.add_argument("--dry-run", action="store_true", help="change nothing")
    prepare.set_defaults(run=cmd_prepare)
    version = commands.add_parser("version", help="print the current version")
    version.set_defaults(run=cmd_version)
    args = parser.parse_args()
    try:
        args.run(args)
    except ReleaseError as error:
        sys.exit(f"error: {error}")


if __name__ == "__main__":
    main()
