// Compiled with `tsc -p test/tsconfig.json`; never run. Each @ts-expect-error
// must be an error, or the build fails, so this pins the typed API.
import { connect, mirrorSelection, type ItemsPage, type Transport } from "../src/index.js";

async function usage() {
  const reasock = await connect({ name: "types", reconnect: true });

  const page: ItemsPage = await reasock.call("items.list", { limit: 5, track: "{GUID}" });
  page.items[0]?.path satisfies string | null | undefined;
  await reasock.call("project.get");
  await reasock.call("action.run", { command: 40044 });
  await reasock.call("action.run", { command: "_SWS_ABOUT" });

  // @ts-expect-error unknown method
  await reasock.call("items.nope");
  // @ts-expect-error takes.list requires { item }
  await reasock.call("takes.list");
  // @ts-expect-error limit is a number
  await reasock.call("items.list", { limit: "5" });
  // @ts-expect-error unknown param
  await reasock.call("items.list", { colour: "red" });

  reasock.on("transport.changed", (state: Transport) => state.playing satisfies boolean);
  reasock.on("items.selectionChanged", (change) => change.items?.[0]?.guid satisfies string | undefined);
  // @ts-expect-error unknown notification
  reasock.on("items.nope", () => {});

  await reasock.subscribe("transport.changed", { event: "items.selectionChanged", options: { include: "all", limit: 50 } });
  // @ts-expect-error unknown event
  await reasock.subscribe("items.nope");
  // @ts-expect-error include is "none" | "changed" | "all"
  await reasock.subscribe({ event: "items.selectionChanged", options: { include: "some" } });

  const items = await mirrorSelection(reasock, "items");
  // @ts-expect-error fields are camelCase
  items.entries.get("{GUID}")?.take_count;
  const item = items.entries.get("{GUID}");
  item?.takeCount satisfies number | undefined;

  await reasock.registerAction("hello", "Hello", (params) => params.context.selectedItems.length);
}
void usage;
