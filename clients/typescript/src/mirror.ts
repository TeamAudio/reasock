import type { ReaSockClient } from "./core.js";
import type { Item, ItemsChange, Track, TracksChange } from "./generated.js";

export interface SelectionMirror<T> {
  /** How many are selected. */
  readonly count: number;
  /** The selection, keyed by GUID. */
  readonly entries: ReadonlyMap<string, T>;
  /** Stops following the selection. */
  stop(): Promise<void>;
}

export interface MirrorOptions<T> {
  /** The most entries per notification before it's truncated. Default 256. */
  limit?: number;
  /** Called after every update. */
  onChange?: (mirror: SelectionMirror<T>) => void;
}

/**
 * Keeps a local copy of the selected items or tracks, keyed by GUID. Applies
 * each selectionChanged notification, and pages through `<kind>.listSelected`
 * when one was truncated.
 */
export async function mirrorSelection(
  client: ReaSockClient,
  kind: "items",
  options?: MirrorOptions<Item>,
): Promise<SelectionMirror<Item>>;
export async function mirrorSelection(
  client: ReaSockClient,
  kind: "tracks",
  options?: MirrorOptions<Track>,
): Promise<SelectionMirror<Track>>;
export async function mirrorSelection(
  client: ReaSockClient,
  kind: "items" | "tracks",
  { limit = 256, onChange }: MirrorOptions<any> = {},
): Promise<SelectionMirror<Item | Track>> {
  let entries = new Map<string, Item | Track>();
  let count = 0;
  let queue = Promise.resolve();

  const mirror: SelectionMirror<Item | Track> = {
    get count() {
      return count;
    },
    get entries() {
      return entries;
    },
    stop: async () => {
      removeHandler();
      await unsubscribe();
    },
  };

  async function resync(): Promise<void> {
    const fresh = new Map<string, Item | Track>();
    let total = 0;
    for (let offset = 0; ; ) {
      const page =
        kind === "items"
          ? await client.call("items.listSelected", { offset, limit: 1000 })
          : await client.call("tracks.listSelected", { offset, limit: 1000 });
      const batch: (Item | Track)[] = "items" in page ? page.items : page.tracks;
      total = page.count;
      batch.forEach((entry) => fresh.set(entry.guid, entry));
      offset += batch.length;
      if (batch.length === 0 || offset >= total) break;
    }
    entries = fresh;
    count = total;
  }

  async function apply(change: ItemsChange | TracksChange): Promise<void> {
    if (change.truncated) return resync();
    for (const guid of change.removed ?? []) entries.delete(guid);
    const changed = "items" in change ? change.items : (change as TracksChange).tracks;
    for (const entry of changed ?? []) entries.set(entry.guid, entry);
    count = change.count;
  }

  const removeHandler = client.on(`${kind}.selectionChanged`, (change: ItemsChange | TracksChange) => {
    // Apply in order, even while a resync is in flight.
    queue = queue.then(() => apply(change)).then(() => onChange?.(mirror));
  });
  const unsubscribe = await client.subscribe({
    event: `${kind}.selectionChanged`,
    options: { include: "changed", limit },
  });
  return mirror;
}
