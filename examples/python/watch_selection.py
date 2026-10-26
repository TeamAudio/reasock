#!/usr/bin/env python3
"""Follows REAPER's item selection and prints it whenever it changes.

Keeps a local copy of the selection from items.selectionChanged, and pages
through items.listSelected when a notification is truncated (more changes
than the subscription's limit).
"""

from reasock import Client, selected_items


def main() -> None:
    with Client("pythonWatch", "1.0") as reasock:
        selection: dict[str, dict] = {}

        def changed(change: dict) -> None:
            if change["truncated"]:
                selection.clear()
                selection.update((item["guid"], item) for item in selected_items(reasock))
            else:
                for guid in change["removed"]:
                    selection.pop(guid, None)
                selection.update((item["guid"], item) for item in change.get("items", []))
            print(f"\n{len(selection)} item(s) selected")
            for item in list(selection.values())[:20]:
                name = item["name"] or "(no take)"
                print(f"  {item['track'] or '?'} / {name} @ {item['position']:.3f}s  {item['path'] or ''}")
            if len(selection) > 20:
                print(f"  ...and {len(selection) - 20} more")

        reasock.on("items.selectionChanged", changed)
        reasock.call("events.subscribe", {"events": [
            {"event": "items.selectionChanged", "options": {"include": "changed", "limit": 200}},
        ]})
        print("Following the selection. Ctrl+C to quit.")
        reasock.run_forever()


if __name__ == "__main__":
    main()
