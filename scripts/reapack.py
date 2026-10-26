#!/usr/bin/env python3
"""ReaPack packaging for ReaSock.

The package lives in TeamAudio/reascripts as Extensions/ReaSock.ext and points
at files attached to this repo's GitHub release, so no binaries go into
reascripts' history.

    # Collect build artifacts into release assets (names match ASSETS below):
    python3 scripts/reapack.py stage --artifacts DIR --out DIR

    # Write the package descriptor into a reascripts checkout:
    python3 scripts/reapack.py descriptor --tag vX.Y.Z --reascripts PATH
"""

from __future__ import annotations

import argparse
import shutil
import sys
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPOSITORY = "TeamAudio/reasock"
PACKAGE = "Extensions/ReaSock.ext"

# (ReaPack options, install path, release asset, where the asset comes from).
# Extension files install to UserPlugins/, [data] files to Data/, and
# [script] files under Scripts/. Binaries share one install name per
# platform, the same name dev-install.sh links, so two copies can't both load.
ASSETS = [
    ("win64", "reaper_reasock.dll", "reaper_reasock-win64.dll", "build"),
    ("linux64", "reaper_reasock.so", "reaper_reasock-linux64.so", "build"),
    ("linux-aarch64", "reaper_reasock.so", "reaper_reasock-linux-aarch64.so", "build"),
    ("darwin-arm64", "reaper_reasock.dylib", "reaper_reasock-darwin-arm64.dylib", "build"),
    ("darwin64", "reaper_reasock.dylib", "reaper_reasock-darwin64.dylib", "build"),
    ("data", "reasock/reasock-api.html", "reasock-api.html", "docs"),
    ("data", "reasock/openrpc.json", "openrpc.json", "docs"),
    ("data", "reasock/licenses/karla-LICENSE.txt", "karla-LICENSE.txt", "docs/licenses"),
    ("data", "reasock/licenses/roboto-slab-LICENSE.txt", "roboto-slab-LICENSE.txt", "docs/licenses"),
    ("script main", "ReaSock/ReaSock_Follow selection.lua", "follow-selection.lua", "examples/lua"),
    ("script main", "ReaSock/ReaSock_Inspector panel.lua", "inspector-panel.lua", "examples/lua"),
    ("script", "ReaSock/json.lua", "json.lua", "examples/lua"),
]

ABOUT = """\
# ReaSock

A bridge between REAPER and other programs. ReaSock opens a local WebSocket
that speaks JSON-RPC 2.0, so tools outside REAPER can:

- **Lend** REAPER actions: register entries in the action list that users can
  bind to keys, menus, and toolbars, and get called when they run
- **Tell**: follow transport, project, and selection changes as they happen
- **Ask**: read tracks, items, takes, and markers

Scripts inside REAPER can use the same protocol through ReaSock_Open,
ReaSock_Call, and ReaSock_Poll.

**ReaSock_Inspector panel** (needs ReaImGui) shows the project, transport,
and selection live, as an example of using ReaSock from a script.

Run **ReaSock: Open API documentation** from the Actions list, or call
`reaper.ReaSock_OpenDocs()`, for the full reference.
"""


class PackagingError(RuntimeError):
    pass


def asset_url(tag: str, asset: str) -> str:
    return f"https://github.com/{REPOSITORY}/releases/download/{tag}/{asset.replace(' ', '%20')}"


def indent(text: str) -> str:
    return textwrap.indent(text.strip(), "  ", lambda line: True)


def descriptor(tag: str, changelog: str) -> str:
    version = tag.removeprefix("v")
    provides = "\n".join(f"  [{options}] {path} {asset_url(tag, asset)}" for options, path, asset, _ in ASSETS)
    return (
        "@description ReaSock: connect REAPER to other programs\n"
        f"@version {version}\n"
        "@author Tech Audio\n"
        # One multi-line @links: reapack-index keeps only the last of
        # repeated @link tags.
        "@links\n"
        f"  GitHub https://github.com/{REPOSITORY}\n"
        "  Tech Audio https://techaud.io\n"
        "@about\n"
        f"{indent(ABOUT)}\n"
        "@changelog\n"
        f"{indent(changelog)}\n"
        "@provides\n"
        f"{provides}\n"
    )


def cmd_stage(args: argparse.Namespace) -> None:
    """Copies every asset, from build artifacts or the repo, into one flat directory."""
    args.out.mkdir(parents=True, exist_ok=True)
    for _, _, asset, origin in ASSETS:
        if origin == "build":
            matches = list(args.artifacts.rglob(asset))
        elif origin.startswith("docs"):
            matches = list((args.artifacts / "reasock-docs").rglob(asset))
        else:
            matches = [ROOT / origin / asset]
        found = [path for path in matches if path.is_file()]
        if len(found) != 1:
            raise PackagingError(f"expected one {asset}, found {len(found)}")
        shutil.copyfile(found[0], args.out / asset)
        print(f"staged {asset}")


def cmd_descriptor(args: argparse.Namespace) -> None:
    changelog_path = args.changelog or ROOT / "releases" / f"changes-{args.tag.removeprefix('v')}.md"
    changelog = Path(changelog_path).read_text(encoding="utf-8")
    text = descriptor(args.tag, changelog)
    if args.reascripts:
        target = args.reascripts / PACKAGE
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
        print(f"wrote {target}")
    else:
        print(text, end="")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    stage = commands.add_parser("stage", help="collect release assets")
    stage.add_argument("--artifacts", type=Path, required=True, help="downloaded workflow artifacts")
    stage.add_argument("--out", type=Path, required=True)
    stage.set_defaults(run=cmd_stage)
    describe = commands.add_parser("descriptor", help="write the ReaPack package descriptor")
    describe.add_argument("--tag", required=True, help="release tag, vX.Y.Z")
    describe.add_argument("--changelog", type=Path, help="default: releases/changes-X.Y.Z.md")
    describe.add_argument("--reascripts", type=Path, help="reascripts checkout to write into (default: print)")
    describe.set_defaults(run=cmd_descriptor)
    args = parser.parse_args()
    try:
        args.run(args)
    except PackagingError as error:
        sys.exit(f"error: {error}")


if __name__ == "__main__":
    main()
