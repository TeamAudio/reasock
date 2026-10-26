#!/usr/bin/env node
// Measures REAPER main-thread time for items.insert and for paging
// items.list on a large project. Adds thousands of items to the current
// project, so use a throwaway one. Writes its own test audio.
//
//   node scripts/bench-items.mjs [items]
//
// Each measurement sends a batch of calls in one message, which REAPER runs
// back to back in one timer tick, and subtracts the time for a batch of
// trivial calls. That leaves the main-thread work, without the wait for the
// next tick.

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect, findInstance, PROTOCOL_VERSION } from "../examples/reasock-client.mjs";

const total = Number(process.argv[2] ?? 16384);
const dir = mkdtempSync(join(tmpdir(), "reasock-bench-"));
const file = join(dir, "blip.wav");
{
  const frames = 4800; // 0.1 s at 48 kHz, mono 16-bit
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + frames * 2, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(48000, 24);
  header.writeUInt32LE(96000, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(frames * 2, 40);
  writeFileSync(file, Buffer.concat([header, Buffer.alloc(frames * 2)]));
}

const reasock = await connect({ name: "benchitems", timeoutMs: 600_000 });

// A raw socket, for sending batches.
const { port, token } = findInstance();
const ws = new WebSocket(`ws://127.0.0.1:${port}`);
await new Promise((resolve) => ws.addEventListener("open", resolve, { once: true }));
let nextId = 1;
function batch(calls) {
  const messages = calls.map(([method, params]) => ({ jsonrpc: "2.0", id: nextId++, method, params }));
  return new Promise((resolve) => {
    ws.addEventListener("message", (event) => resolve(JSON.parse(event.data)), { once: true });
    ws.send(JSON.stringify(messages));
  });
}
await batch([["initialize", { protocolVersion: PROTOCOL_VERSION, client: { name: "benchraw" }, token }]]);

/** Milliseconds of main-thread work per call, the median of `runs` batches of `size`. */
async function perCall(method, params, { size = 10, runs = 7 } = {}) {
  const time = async (calls) => {
    const started = performance.now();
    const replies = await batch(calls);
    const failed = replies.find((reply) => reply.error);
    if (failed) throw new Error(`${failed.error.code} ${failed.error.message}`);
    return performance.now() - started;
  };
  const samples = [];
  for (let run = 0; run < runs; run++) {
    const work = await time(Array.from({ length: size }, () => [method, params]));
    const idle = await time(Array.from({ length: size }, () => ["transport.get", {}]));
    samples.push((work - idle) / size);
  }
  samples.sort((a, b) => a - b);
  return samples[Math.floor(samples.length / 2)];
}

// Build the project with items.insert, 1000 files a call, a track at a time.
const before = (await reasock.call("items.list", { limit: 1 })).count;
let inserted = 0;
const insertTimes = [];
while (inserted < total) {
  const count = Math.min(1000, total - inserted);
  const started = performance.now();
  await reasock.call("items.insert", {
    files: Array.from({ length: count }, () => ({ path: file })),
    position: 0,
    track: "new",
    select: false,
    gap: 0.05,
  });
  insertTimes.push((performance.now() - started) / count);
  inserted += count;
}
const items = (await reasock.call("items.list", { limit: 1 })).count;
console.log(`project: ${items} items (${items - before} inserted)`);
console.log(`items.insert: ${(insertTimes.reduce((a, b) => a + b) / insertTimes.length).toFixed(3)} ms per file (round trip, 1000 files per call)`);

/** Main-thread milliseconds per call, as REAPER measured it, over `runs` calls. */
async function measured(method, params, runs = 20) {
  await reasock.call("debug.stats", { reset: true });
  for (let run = 0; run < runs; run++) await reasock.call(method, params);
  const { methods } = await reasock.call("debug.stats");
  return { avg: methods[method].avgUs / 1000, max: methods[method].maxUs / 1000 };
}

const pages = [
  ["items.list, 1000 items", { limit: 1000 }],
  ["items.list, 1000 items, offset 15000", { offset: Math.max(0, items - 1000), limit: 1000 }],
  ["items.list, 1000 items + 2 ext keys", { limit: 1000, ext: ["bench.a", "bench.b"] }],
  ["items.list, 1000 items + notes", { limit: 1000, notes: true }],
  ["items.list, 100 items", { limit: 100 }],
];
for (const [label, params] of pages) {
  const { avg, max } = await measured("items.list", params);
  const roundTrip = await perCall("items.list", params);
  console.log(`${label}: ${avg.toFixed(2)} ms main thread (max ${max.toFixed(2)}); ${roundTrip.toFixed(2)} ms per call end to end`);
}
const big = await measured("items.list", { limit: 10000 }, 3);
console.log(`whole project (${items} items): ~${((big.avg * items) / 10000).toFixed(0)} ms main thread`);
const insert = await measured("items.insert", { files: [{ path: file }], position: 0, select: false }, 20);
console.log(`items.insert, one file: ${insert.avg.toFixed(2)} ms main thread`);

ws.close();
reasock.close();
rmSync(dir, { recursive: true, force: true });
