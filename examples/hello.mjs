#!/usr/bin/env node
// Registers a "Hello" action in REAPER and logs whatever arrives when it runs.
//
//   node examples/hello.mjs              # wait for you to trigger the action
//   node examples/hello.mjs --self-test  # select all items, trigger it, exit

import { connect } from "./reasock-client.mjs";

const selfTest = process.argv.includes("--self-test");

const reasock = await connect({ name: "demo", version: "0.1.0" });
const { session } = reasock;
console.log(
  `connected to REAPER ${session.reaper.version} (ReaSock ${session.server.version}, protocol ${session.protocolVersion})`,
);

reasock.on("action.triggered", ({ command, context }) => {
  console.log(`→ ${command} fired`);
  for (const item of context.selectedItems) {
    console.log(`   ${item.track ?? "(unnamed track)"} @ ${item.position.toFixed(3)}s: ${item.path ?? "(no file)"}`);
  }
  if (context.selectedItems.length === 0) console.log("   (no items selected)");
  if (context.truncated) console.log(`   ...and ${context.selectedItemCount - context.selectedItems.length} more`);
  if (selfTest) reasock.close();
});
reasock.ws.addEventListener("close", () => process.exit(0));

const action = await reasock.call("action.register", { id: "hello", label: "Hello" });
console.log(`registered ${action.command} (command ${action.commandId})`);

if (selfTest) {
  await reasock.call("action.run", { command: 40182 }); // Item: Select all items
  await reasock.call("action.run", { command: action.command });
} else {
  console.log('waiting... find "ReaSock (demo): Hello" in the Actions list (Ctrl+C to quit)');
}
