#!/usr/bin/env node
// Checks the methods that change the project (items.insert, items.set,
// takes.set) against a running REAPER with ReaSock loaded. Run it on a
// scratch project: it adds items and tracks, then undoes each change and
// checks the undo too. Writes its own test audio to a temporary folder.
//
//   node scripts/edit-test.mjs

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "../examples/reasock-client.mjs";

let failures = 0;
async function check(name, fn) {
  try {
    await fn();
    console.log(`ok   ${name}`);
  } catch (error) {
    failures++;
    console.log(`FAIL ${name}\n     ${error.stack.split("\n").slice(0, 3).join("\n     ")}`);
  }
}

/** A mono 16-bit WAV of `seconds` of silence. */
function wav(path, seconds, rate = 48000) {
  const frames = Math.round(seconds * rate);
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + frames * 2, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(frames * 2, 40);
  writeFileSync(path, Buffer.concat([header, Buffer.alloc(frames * 2)]));
  return path;
}

const dir = mkdtempSync(join(tmpdir(), "reasock-edit-"));
const long = wav(join(dir, "long.wav"), 4);
const short = wav(join(dir, "short.wav"), 1.5);
const notMedia = join(dir, "not-media.wav");
writeFileSync(notMedia, "not audio");

const client = await connect({ name: "edittest" });
const close = (a, b, message) => assert.ok(Math.abs(a - b) < 1e-6, `${message ?? ""} ${a} != ${b}`);
const run = (command) => client.call("action.run", { command });
const undo = () => run(40029);
const redo = () => run(40030);
const counts = async () => ({
  items: (await client.call("items.list", { limit: 1 })).count,
  tracks: (await client.call("tracks.list", { limit: 1 })).count,
});
const lastUndo = async () => (await client.call("project.get"), (await poll("project.edited")).lastUndo);
let edits;
async function poll(event) {
  // The last project.edited notification, after REAPER's next tick.
  await new Promise((resolve) => setTimeout(resolve, 150));
  return edits;
}
client.on("project.edited", (params) => (edits = params));
await client.call("events.subscribe", { events: ["project.edited"] });

await check("initialize lists the methods the server supports", async () => {
  for (const method of ["items.insert", "items.set", "takes.set", "items.list"]) {
    assert.ok(client.session.capabilities.methods.includes(method), method);
  }
  assert.equal(client.session.protocolVersion, "0.5.0");
});

await check("transport.get has the edit cursor and time selection", async () => {
  const transport = await client.call("transport.get");
  assert.equal(typeof transport.editCursor, "number");
  assert.ok(transport.timeSelection === null || transport.timeSelection.end > transport.timeSelection.start);
});

await check("items.insert places a sequence on a new, named track, as one undo point", async () => {
  const before = await counts();
  const { items } = await client.call("items.insert", {
    files: [
      { path: long, name: "first", notes: "hello", ext: { "edittest.id": "a1" } },
      { path: short, sourceStart: 0.5, length: 0.75 },
    ],
    position: 10,
    track: "new",
    trackName: "Edit test",
    gap: 0.5,
  });
  assert.equal(items.length, 2);
  close(items[0].position, 10);
  close(items[0].length, 4);
  close(items[1].position, 14.5, "gap");
  close(items[1].length, 0.75);
  close(items[1].startOffset, 0.5);
  assert.equal(items[0].name, "first");
  assert.equal(items[1].name, "short.wav");
  assert.equal(items[0].path, long);
  assert.equal(items[0].track, "Edit test");
  assert.equal(items[0].trackGuid, items[1].trackGuid);
  assert.equal(typeof items[0].takeGuid, "string");
  assert.equal(items[0].playRate, 1);
  const { items: found } = await client.call("items.get", {
    guids: items.map((item) => item.guid),
    notes: true,
    ext: ["edittest.id", "edittest.missing"],
  });
  assert.equal(found[0].notes, "hello");
  assert.deepEqual(found[0].ext, { "edittest.id": "a1", "edittest.missing": null });
  assert.deepEqual(found[1].ext, { "edittest.id": null, "edittest.missing": null });
  const selected = await client.call("items.listSelected");
  assert.deepEqual(selected.items.map((item) => item.guid), items.map((item) => item.guid));
  assert.equal(await lastUndo(), "Insert media (ReaSock: edittest)");
  await undo();
  assert.deepEqual(await counts(), before, "undo removes the items and the new track");
});

await check("items.insert defaults to the selected track, at the edit cursor", async () => {
  const { tracks } = await client.call("tracks.list", { limit: 1 });
  await run(40297); // Track: Unselect all tracks
  await client.call("items.insert", { files: [{ path: short }], track: tracks[0].guid, position: 0 });
  await undo();
  await run(40296); // Track: Select all tracks
  const cursor = (await client.call("transport.get")).editCursor;
  const { items } = await client.call("items.insert", { files: [{ path: short }], moveCursor: true });
  assert.equal(items[0].trackGuid, tracks[0].guid);
  close(items[0].position, cursor);
  close((await client.call("transport.get")).editCursor, cursor + 1.5, "moveCursor");
  await undo();
  await client.call("action.run", { command: 40042 }); // back to the project start
});

await check("track: selected is strict, current falls back to the last touched track", async () => {
  await run(40297); // Track: Unselect all tracks
  await assert.rejects(client.call("items.insert", { files: [{ path: short }], track: "selected" }), {
    code: -32014,
  });
  const before = await counts();
  const { items } = await client.call("items.insert", { files: [{ path: short }], position: 30 });
  assert.equal((await counts()).tracks, before.tracks, "uses an existing track");
  assert.ok(items[0].trackGuid);
  await undo();
});

await check("items.insert arranges files on tracks or as takes", async () => {
  const before = await counts();
  const { tracks } = await client.call("tracks.list", { limit: 1 });
  const onTracks = await client.call("items.insert", {
    files: [{ path: long }, { path: short }],
    track: tracks[0].guid,
    arrange: "tracks",
    position: 40,
  });
  assert.equal((await counts()).tracks, before.tracks + 2);
  assert.notEqual(onTracks.items[0].trackGuid, onTracks.items[1].trackGuid);
  assert.notEqual(onTracks.items[0].trackGuid, tracks[0].guid);
  close(onTracks.items[1].position, 40);
  const [first] = (await client.call("tracks.get", { guids: [onTracks.items[0].trackGuid] })).tracks;
  assert.equal(first.number, 2, "new tracks go right after the target track");
  await undo();

  const asTakes = await client.call("items.insert", {
    files: [{ path: short, name: "variation 1", ext: { "edittest.id": "v" } }, { path: long, name: "variation 2" }],
    arrange: "takes",
    position: 40,
  });
  assert.equal(asTakes.items.length, 1);
  const [item] = asTakes.items;
  assert.equal(item.takeCount, 2);
  close(item.length, 1.5, "as long as the first file");
  assert.equal(item.path, short, "the first file's take is active");
  const { takes } = await client.call("takes.list", { item: item.guid });
  assert.deepEqual(takes.map((take) => take.name), ["variation 1", "variation 2"]);
  const [tagged] = (await client.call("items.get", { guids: [item.guid], ext: ["edittest.id"] })).items;
  assert.equal(tagged.ext["edittest.id"], "v", "ext goes on the one new item");
  await undo();
  await assert.rejects(
    client.call("items.insert", { files: [{ path: short }, { path: long, notes: "x" }], arrange: "takes" }),
    (error) => error.code === -32602 && error.data.index === 1,
  );
  assert.deepEqual(await counts(), before);
});

await check("items.insert checks every file first, and says which one failed", async () => {
  const before = await counts();
  await assert.rejects(
    client.call("items.insert", { files: [{ path: long }, { path: join(dir, "missing.wav") }], track: "new" }),
    (error) => error.code === -32015 && error.data.index === 1 && error.data.reason === "no such file",
  );
  await assert.rejects(
    client.call("items.insert", { files: [{ path: notMedia }] }),
    (error) => error.code === -32015 && error.data.index === 0,
  );
  await assert.rejects(client.call("items.insert", { files: [{ path: "relative.wav" }] }), { code: -32602 });
  await assert.rejects(client.call("items.insert", { files: [{ path: long, sourceStart: 4 }] }), { code: -32602 });
  await assert.rejects(
    client.call("items.insert", { files: [{ path: short }, { path: long, length: 4.01 }] }),
    (error) => error.code === -32602 && error.data.index === 1 && error.data.path === long,
  );
  await assert.rejects(client.call("items.insert", { files: [] }), { code: -32602 });
  await assert.rejects(client.call("items.insert", { files: [{ path: long, ext: { "bad key": "x" } }] }), {
    code: -32602,
  });
  assert.deepEqual(await counts(), before, "nothing was inserted");
});

await check("a range just past the end of the file is shortened to fit", async () => {
  const { items } = await client.call("items.insert", {
    files: [{ path: long, sourceStart: 1, length: 3.0005 }],
    position: 50,
  });
  close(items[0].length, 3);
  await undo();
});

await check("items.set and takes.set write notes, names, and ext as one undo point", async () => {
  const { items } = await client.call("items.insert", { files: [{ path: short }], position: 60 });
  const [item] = items;
  await client.call("items.set", {
    items: [{ guid: item.guid, notes: "note", ext: { "edittest.a": "1", "edittest.b": "2" } }],
  });
  assert.equal(await lastUndo(), "Set item properties (ReaSock: edittest)");
  await client.call("items.set", { items: [{ guid: item.guid, ext: { "edittest.b": null } }], undo: "Tag it" });
  assert.equal(await lastUndo(), "Tag it");
  const get = async () =>
    (await client.call("items.get", { guids: [item.guid], notes: true, ext: ["edittest.a", "edittest.b"] }))
      .items[0];
  assert.deepEqual((await get()).ext, { "edittest.a": "1", "edittest.b": null }, "null removes a key");
  await undo();
  assert.deepEqual((await get()).ext, { "edittest.a": "1", "edittest.b": "2" }, "undo brings it back");
  await redo();
  assert.deepEqual((await get()).ext, { "edittest.a": "1", "edittest.b": null }, "redo removes it again");
  const listed = await client.call("items.list", { limit: 10000, ext: ["edittest.a"] });
  assert.equal(listed.items.find((entry) => entry.guid === item.guid).ext["edittest.a"], "1");

  await client.call("takes.set", { takes: [{ guid: item.takeGuid, name: "renamed", ext: { "edittest.t": "x" } }] });
  const { takes } = await client.call("takes.list", { item: item.guid, ext: ["edittest.t"] });
  assert.equal(takes[0].name, "renamed");
  assert.equal(takes[0].ext["edittest.t"], "x");

  await assert.rejects(
    client.call("items.set", {
      items: [{ guid: item.guid, notes: "changed" }, { guid: "{00000000-0000-0000-0000-000000000000}", notes: "x" }],
    }),
    { code: -32014 },
  );
  assert.equal((await get()).notes, "note", "an unknown GUID changes nothing");
  await assert.rejects(
    client.call("items.set", { items: [{ guid: item.guid, ext: { "edittest.big": "x".repeat(65537) } }] }),
    { code: -32602 },
  );
  for (let i = 0; i < 5; i++) await undo();
});

await check("takes.set points a take at another file, as one undo point", async () => {
  const { items } = await client.call("items.insert", { files: [{ path: short, sourceStart: 0.25, length: 1 }], position: 70 });
  const [item] = items;
  await client.call("takes.set", { takes: [{ guid: item.takeGuid, path: long }] });
  let [now] = (await client.call("items.get", { guids: [item.guid] })).items;
  assert.equal(now.path, long);
  close(now.startOffset, 0.25, "keeps its offset");
  close(now.length, 1, "keeps its length");
  assert.equal(await lastUndo(), "Set take properties (ReaSock: edittest)");
  await undo();
  [now] = (await client.call("items.get", { guids: [item.guid] })).items;
  assert.equal(now.path, short, "undo puts the old file back");
  await assert.rejects(
    client.call("takes.set", { takes: [{ guid: item.takeGuid, name: "x" }, { guid: item.takeGuid, path: notMedia }] }),
    (error) => error.code === -32015 && error.data.index === 1,
  );
  assert.equal((await client.call("takes.list", { item: item.guid })).takes[0].name, "short.wav", "nothing changed");
  await undo();
});

await check("position: play is the play position while playing, else the edit cursor", async () => {
  const cursor = (await client.call("transport.get")).editCursor;
  const stopped = await client.call("items.insert", { files: [{ path: short }], position: "play" });
  close(stopped.items[0].position, cursor);
  await undo();
  await run(1007); // Transport: Play
  await new Promise((resolve) => setTimeout(resolve, 600));
  const playing = await client.call("items.insert", { files: [{ path: short }], position: "play", track: "new" });
  await run(1016); // Transport: Stop
  assert.ok(playing.items[0].position > cursor + 0.05, `${playing.items[0].position} is past the cursor`);
  await undo();
});

await check("toggle actions report their state to REAPER", async () => {
  await client.call("action.register", { id: "toggled", label: "Toggled", toggle: true });
  await client.call("action.register", { id: "plain", label: "Plain" });
  // What REAPER itself reports, through a ReaScript (macOS can hand one to the
  // running REAPER with `open`): -1 no state, 0 off, 1 on.
  const reaperStates = async () => {
    if (process.platform !== "darwin") return null;
    const script = join(dir, "toggle-state.lua");
    const result = join(dir, "toggle-state.txt");
    rmSync(result, { force: true });
    writeFileSync(
      script,
      `local f = io.open(${JSON.stringify(result)}, "w")
for _, id in ipairs({"toggled", "plain"}) do
  f:write(reaper.GetToggleCommandStateEx(0, reaper.NamedCommandLookup("_REASOCK_edittest_" .. id)), " ")
end
f:write("done") f:close()`,
    );
    execFileSync("open", ["-a", "REAPER", script]);
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline && !(existsSync(result) && readFileSync(result, "utf8").includes("done"))) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return readFileSync(result, "utf8").trim().split(" ").slice(0, 2).map(Number);
  };
  const skipIfNoLua = (states, expected, message) => states === null || assert.deepEqual(states, expected, message);
  skipIfNoLua(await reaperStates(), [0, -1]);
  await client.call("action.setState", { id: "toggled", on: true });
  skipIfNoLua(await reaperStates(), [1, -1]);
  await client.call("action.register", { id: "toggled", label: "Toggled", toggle: true });
  skipIfNoLua(await reaperStates(), [1, -1], "registering again keeps the state");
  await assert.rejects(client.call("action.setState", { id: "plain", on: true }), { code: -32602 });
  await assert.rejects(client.call("action.setState", { id: "missing", on: true }), { code: -32011 });
  await client.call("action.unregister", { id: "toggled" });
  await client.call("action.unregister", { id: "plain" });
});

client.close();
rmSync(dir, { recursive: true, force: true });
console.log(failures ? `\n${failures} failed` : "\nall passed");
process.exit(failures ? 1 : 0);
