#!/usr/bin/env node
// Places media files at REAPER's edit cursor, the way a sound library app
// would: on the current track, one after another, as one undo point. Tags
// each new item with where it came from, using extension data.
//
//   node examples/place.mjs sound.wav [more.wav ...]
//   node examples/place.mjs --from 1.5 --length 0.8 sound.wav   # just part of it

import { resolve } from "node:path";
import { connect } from "./reasock-client.mjs";

const args = process.argv.slice(2);
const option = (name) => {
  const at = args.indexOf(name);
  return at === -1 ? undefined : Number(args.splice(at, 2)[1]);
};
const sourceStart = option("--from");
const length = option("--length");
if (args.length === 0) {
  console.error("usage: node examples/place.mjs [--from seconds] [--length seconds] file...");
  process.exit(1);
}

const reasock = await connect({ name: "place" });
if (!reasock.session.capabilities.methods?.includes("items.insert")) {
  console.error("This ReaSock is too old to insert media; update it through ReaPack.");
  process.exit(1);
}

try {
  const { items } = await reasock.call("items.insert", {
    files: args.map((path) => ({
      path: resolve(path),
      sourceStart,
      length,
      ext: { "place.source": resolve(path) },
    })),
    moveCursor: true,
  });
  for (const item of items) {
    console.log(`${item.name} on ${item.track ?? "(unnamed track)"} at ${item.position.toFixed(3)}s, ${item.length.toFixed(3)}s long`);
  }
} catch (error) {
  // MediaUnavailable says which file REAPER couldn't use.
  console.error(error.data ? `${error.data.path}: ${error.data.reason}` : error.message);
  process.exitCode = 1;
}
reasock.close();
