-- Prints the selected items to the ReaScript console whenever the selection
-- changes, using ReaSock's items.selectionChanged event from inside REAPER.
--
-- Run it from the Actions list (ReaScript: Load...). Stop it with the
-- "Terminate instances" prompt or by closing REAPER.

local dir = debug.getinfo(1, "S").source:match("^@(.*[/\\])") or ""
local json = dofile(dir .. "json.lua")

local conn = reaper.ReaSock_Open("followSelection")
if conn == 0 then
  reaper.ShowMessageBox("Couldn't open a ReaSock connection. Is ReaSock installed?", "ReaSock", 0)
  return
end
reaper.atexit(function() reaper.ReaSock_Close(conn) end)

-- Ask for the whole selection with every notification, up to 50 items.
local reply = json.decode(reaper.ReaSock_Call(conn, "events.subscribe", json.encode({
  events = { { event = "items.selectionChanged", options = { include = "all", limit = 50 } } },
})))
if reply.error then
  reaper.ShowMessageBox(reply.error.message, "ReaSock", 0)
  return
end

local function show(change)
  reaper.ClearConsole()
  reaper.ShowConsoleMsg(("%d item(s) selected\n"):format(change.count))
  if change.truncated then
    reaper.ShowConsoleMsg("(too many to list)\n")
    return
  end
  for _, item in ipairs(change.items) do
    reaper.ShowConsoleMsg(("  %s / %s @ %.3fs  %s\n"):format(
      item.track or "?", item.name or "(no take)", item.position, item.path or ""))
  end
end

local function loop()
  while true do
    local text = reaper.ReaSock_Poll(conn)
    if text == "" then break end
    local message = json.decode(text)
    if message.method == "items.selectionChanged" then show(message.params) end
  end
  reaper.defer(loop)
end

loop()
