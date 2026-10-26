#!/usr/bin/env node
// Measures how much of REAPER's main thread Tell uses as the item selection
// grows. Needs a running REAPER whose current project has at least one item;
// it duplicates items until each size is reached, so use a throwaway project.
//
//   node scripts/bench-events.mjs [maxItems]

import { connect } from "../examples/reasock-client.mjs";

const SELECT_ALL_ITEMS = 40182;
const DUPLICATE_ITEMS = 41295;
const maxItems = Number(process.argv[2] ?? 16384);
const sizes = [1, 16, 256, 1024, 4096, 16384, 65536].filter((size) => size <= maxItems);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const reasock = await connect({ name: "bench", timeoutMs: 120_000 });
const allEvents = reasock.session.capabilities.events.available;

let selected = 0;
let selectionMessages = 0;
reasock.on("items.selectionChanged", ({ count }) => {
  selected = count;
  selectionMessages++;
});

/** Waits until `condition` holds, or gives up quietly. */
async function until(condition, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs;
  while (!condition() && Date.now() < deadline) await sleep(20);
}

/** Waits until a new items.selectionChanged arrives, or gives up quietly. */
async function nextSelection(timeoutMs = 60_000) {
  const before = selectionMessages;
  const deadline = Date.now() + timeoutMs;
  while (selectionMessages === before && Date.now() < deadline) await sleep(20);
}

await reasock.call("events.subscribe", { events: allEvents });
await nextSelection(5000);
await reasock.call("action.run", { command: SELECT_ALL_ITEMS });
await sleep(500);
if (selected === 0) throw new Error("the current project has no items to duplicate");

const rows = [];
for (const size of sizes) {
  let change = null;
  while (selected < size) {
    const before = selected;
    await reasock.call("debug.stats", { reset: true });
    await reasock.call("action.run", { command: DUPLICATE_ITEMS });
    await reasock.call("action.run", { command: SELECT_ALL_ITEMS });
    // Duplicating first selects just the copies, so wait for the full count.
    await until(() => selected > before);
    if (selected <= before) throw new Error(`action ${DUPLICATE_ITEMS} did not duplicate items`);
    // The last doubling: a change adding as many items as were selected.
    change = await reasock.call("debug.stats", { reset: true });
  }

  // A full send: re-subscribing makes ReaSock serialize the whole selection.
  await reasock.call("events.unsubscribe", { events: ["items.selectionChanged"] });
  await sleep(200);
  await reasock.call("debug.stats", { reset: true });
  await reasock.call("events.subscribe", { events: ["items.selectionChanged"] });
  await nextSelection();
  const send = await reasock.call("debug.stats", { reset: true });

  // Idle: nothing changes, so this is the cost of watching.
  await sleep(3000);
  const idle = await reasock.call("debug.stats", { reset: true });

  const items = idle.events["items.selectionChanged"];
  rows.push({
    items: selected,
    "idle tick avg ms": idle.ticks.avgUs / 1000,
    "idle tick max ms": idle.ticks.maxUs / 1000,
    "items check avg ms": items.check.avgUs / 1000,
    "items snapshots": items.snapshot.count,
    "items compare avg ms": items.compare.avgUs / 1000,
    "send tick ms": send.ticks.maxUs / 1000,
    "payload KB": Math.round(send.events["items.selectionChanged"].lastBytes / 1024),
    "big change tick ms": change ? change.ticks.maxUs / 1000 : null,
    "big change KB": change ? Math.round((change.events["items.selectionChanged"]?.lastBytes ?? 0) / 1024) : null,
    "ticks/s": Math.round(idle.ticks.count / idle.seconds),
  });
  console.log(`measured ${selected} items`);
}

console.table(rows);
reasock.close();
process.exit(0);
