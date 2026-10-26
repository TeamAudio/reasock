#!/usr/bin/env python3
"""Adds "Copy file paths of selected items" to REAPER's action list.

Bind it to a key (Actions > Show action list, search "copy file paths"),
select some items, and press it: their source files' paths land on the
clipboard, one per line. Keep this running while you work.

    python3 copy_paths.py           # copy to the clipboard
    python3 copy_paths.py --print   # print instead
"""

import shutil
import subprocess
import sys

from reasock import Client, selected_items

ACTION_ID = "copyPaths"


def copy_to_clipboard(text: str) -> bool:
    for command in (["pbcopy"], ["clip"], ["wl-copy"], ["xclip", "-selection", "clipboard"]):
        if shutil.which(command[0]):
            subprocess.run(command, input=text.encode(), check=True)
            return True
    return False


def main() -> None:
    print_only = "--print" in sys.argv
    with Client("pythonExamples", "1.0") as reasock:
        action = reasock.call(
            "action.register", {"id": ACTION_ID, "label": "Copy file paths of selected items"}
        )

        def triggered(params: dict) -> None:
            if params["id"] != ACTION_ID:
                return
            items = selected_items(reasock, params["context"])
            # Unique, in selection order; MIDI and empty items have no file.
            paths = list(dict.fromkeys(item["path"] for item in items if item["path"]))
            text = "\n".join(paths)
            if print_only or not copy_to_clipboard(text):
                print(text)
            print(f"{len(paths)} path(s) from {len(items)} selected item(s)", file=sys.stderr)

        reasock.on("action.triggered", triggered)
        print(f"Registered {action['command']} in REAPER's action list. Ctrl+C to quit.",
              file=sys.stderr)
        reasock.run_forever()


if __name__ == "__main__":
    main()
