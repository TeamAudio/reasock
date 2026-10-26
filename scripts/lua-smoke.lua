-- Checks the ReaScript API from inside REAPER. Writes one "ok" or "FAIL"
-- line per check to ~/.reasock/lua-smoke.txt, then "done".
-- scripts/smoke-test.mjs runs it on macOS.

local dir = debug.getinfo(1, "S").source:match("^@(.*[/\\])") or ""
local json = dofile(dir .. "../examples/lua/json.lua")
local path = os.getenv("HOME") .. "/.reasock/lua-smoke.txt"
local out = io.open(path, "w")

local function check(name, ok, detail)
  out:write((ok and "ok   " or "FAIL ") .. name .. ((not ok and detail) and (" -- " .. detail) or "") .. "\n")
  out:flush()
end

local function call(conn, method, params)
  return json.decode(reaper.ReaSock_Call(conn, method, params and json.encode(params) or ""))
end

check("Open rejects a bad client name", reaper.ReaSock_Open("has space") == 0)
local conn = reaper.ReaSock_Open("luasmoke")
check("Open returns a connection", conn > 0, tostring(conn))

local project = call(conn, "project.get")
check("Call returns a result", project.result ~= nil and type(project.result.dirty) == "boolean")
local bad = call(conn, "tracks.list", { limit = 0 })
check("Call returns errors", bad.error ~= nil and bad.error.code == -32602)
local parse = json.decode(reaper.ReaSock_Call(conn, "tracks.list", "{oops"))
check("Call rejects bad JSON params", parse.error ~= nil and parse.error.code == -32700)
local unknown = json.decode(reaper.ReaSock_Call(123, "project.get", ""))
check("Call rejects unknown connections", unknown.error ~= nil and unknown.error.code == -32600)

local registered = call(conn, "action.register", { id = "ping", label = "Lua smoke test" })
check("scripts can register actions", registered.result and registered.result.command == "_REASOCK_luasmoke_ping")
check("Send accepts a request", reaper.ReaSock_Send(conn, json.encode({
  jsonrpc = "2.0", id = 7, method = "events.subscribe", params = { events = { "transport.changed" } },
})))
check("Send refuses unknown connections", not reaper.ReaSock_Send(123, "{}"))

local seen = {}
local frames = 0
local function loop()
  frames = frames + 1
  if frames == 3 then call(conn, "action.run", { command = "_REASOCK_luasmoke_ping" }) end
  while true do
    local text = reaper.ReaSock_Poll(conn)
    if text == "" then break end
    local message = json.decode(text)
    seen[message.method or ("reply " .. tostring(message.id))] = message
  end
  if frames < 20 then
    reaper.defer(loop)
    return
  end
  check("Poll returns queued replies", seen["reply 7"] and seen["reply 7"].result ~= nil)
  check("Poll returns subscribed events", seen["transport.changed"] ~= nil)
  check("Poll returns triggers for the script's actions",
    seen["action.triggered"] and seen["action.triggered"].params.id == "ping")
  reaper.ReaSock_Close(conn)
  check("Close ends the connection", reaper.ReaSock_Poll(conn) == "" and not reaper.ReaSock_Send(conn, "{}"))
  out:write("done\n")
  out:close()
end
loop()
