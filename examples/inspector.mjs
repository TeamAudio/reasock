#!/usr/bin/env node
// Follows REAPER as you work: prints the current project, transport, and
// selections, then every change as it happens.
//
//   node examples/inspector.mjs                         # everything
//   node examples/inspector.mjs items.selectionChanged  # just these events

import { connect } from "./reasock-client.mjs";

const reasock = await connect({ name: "inspector", version: "0.1.0" });
const available = reasock.session.capabilities.events.available;
const wanted = process.argv.slice(2).length ? process.argv.slice(2) : available;

const time = () => new Date().toTimeString().slice(0, 8) + `.${String(Date.now() % 1000).padStart(3, "0")}`;
const seconds = (value) => `${value.toFixed(3)}s`;

/** Summarizes a collection change: the count, what changed, and changed entries. */
function describeChange({ count, added, removed, changed, truncated }, entries = [], line) {
  const summary = truncated
    ? `${count} selected (too many changes to list)`
    : `${count} selected (+${added.length} -${removed.length} ~${changed.length})`;
  return summary + entries.map((entry) => `\n    ${line(entry)}`).join("");
}

const describe = {
  "transport.changed": ({ playing, paused, recording }) =>
    recording ? "● recording" : paused ? "❚❚ paused" : playing ? "▶ playing" : "■ stopped",
  "project.changed": ({ tab, name, dirty }) => `tab ${tab}: ${name ?? "(unsaved)"}${dirty ? " *" : ""}`,
  "project.edited": ({ stateChangeCount, lastUndo }) => `#${stateChangeCount} ${lastUndo ?? ""}`,
  "items.selectionChanged": (change) =>
    describeChange(change, change.items, (item) =>
      `${item.track ?? "?"} / ${item.name ?? "(no take)"} @ ${seconds(item.position)} +${seconds(item.length)}  ${item.path ?? ""}`),
  "tracks.selectionChanged": (change) =>
    describeChange(change, change.tracks, (track) => `${track.number}: ${track.name || "(unnamed)"}`),
};

for (const event of wanted) {
  reasock.on(event, (params) => console.log(`${time()}  ${event.padEnd(24)} ${describe[event](params)}`));
}
reasock.ws.addEventListener("close", () => {
  console.log("REAPER closed the connection");
  process.exit(0);
});

// Collection events: show changed entries, but at most 20 per notification.
const subscriptions = wanted.map((event) =>
  event.endsWith(".selectionChanged") ? { event, options: { include: "changed", limit: 20 } } : event,
);
await reasock.call("events.subscribe", { events: subscriptions });
console.log(`following REAPER ${reasock.session.reaper.version} (Ctrl+C to quit)\n`);
