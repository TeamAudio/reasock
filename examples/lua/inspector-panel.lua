-- A dockable ReaImGui panel that follows REAPER through ReaSock: the
-- project, transport, last edit, and the selected items and tracks, updated
-- as they change.
--
-- Needs ReaImGui 0.10 or later (available through ReaPack).

local dir = debug.getinfo(1, "S").source:match("^@(.*[/\\])") or ""
local json = dofile(dir .. "json.lua")

if not reaper.ImGui_GetBuiltinPath then
  reaper.ShowMessageBox("This script needs ReaImGui, available through ReaPack.", "ReaSock", 0)
  return
end
package.path = reaper.ImGui_GetBuiltinPath() .. "/?.lua"
local ImGui = require "imgui" "0.10"

local conn = reaper.ReaSock_Open and reaper.ReaSock_Open("inspectorPanel")
if not conn or conn == 0 then
  reaper.ShowMessageBox("Couldn't open a ReaSock connection. Is ReaSock installed?", "ReaSock", 0)
  return
end
reaper.atexit(function() reaper.ReaSock_Close(conn) end)

-- Latest state, filled in by notifications.
local state = {
  project = nil,
  transport = nil,
  edit = nil,
  items = { count = 0, entries = {}, truncated = false },
  tracks = { count = 0, entries = {}, truncated = false },
}

local subscribe = json.decode(reaper.ReaSock_Call(conn, "events.subscribe", json.encode({
  events = {
    "project.changed",
    "project.edited",
    "transport.changed",
    -- "all": every notification carries the whole selection (up to limit).
    { event = "items.selectionChanged", options = { include = "all", limit = 500 } },
    { event = "tracks.selectionChanged", options = { include = "all", limit = 100 } },
  },
})))
if subscribe.error then
  reaper.ShowMessageBox(subscribe.error.message, "ReaSock", 0)
  return
end

local function apply(message)
  local params = message.params
  if message.method == "project.changed" then
    state.project = params
  elseif message.method == "project.edited" then
    state.edit = params
  elseif message.method == "transport.changed" then
    state.transport = params
  elseif message.method == "items.selectionChanged" then
    state.items = { count = params.count, entries = params.items or {}, truncated = params.truncated }
  elseif message.method == "tracks.selectionChanged" then
    state.tracks = { count = params.count, entries = params.tracks or {}, truncated = params.truncated }
  end
end

local function poll()
  while true do
    local text = reaper.ReaSock_Poll(conn)
    if text == "" then return end
    local message = json.decode(text)
    if message.method then apply(message) end
  end
end

local GRAY, GREEN, RED, AMBER = 0x9CA3AFFF, 0x4ADE80FF, 0xF87171FF, 0xFBBF24FF

local function transport_label(transport)
  if not transport then return "…", GRAY end
  if transport.recording then return "Recording", RED end
  if transport.paused then return "Paused", AMBER end
  if transport.playing then return "Playing", GREEN end
  return "Stopped", GRAY
end

local function file_name(path)
  return path and path:match("[^/\\]+$") or ""
end

local function draw(ctx)
  local project = state.project
  local name = project and (project.name or "(unsaved)") or "…"
  ImGui.Text(ctx, ("%s%s"):format(name, project and project.dirty and " *" or ""))
  ImGui.SameLine(ctx)
  local label, color = transport_label(state.transport)
  ImGui.TextColored(ctx, color, label)
  if state.edit and state.edit.lastUndo then
    ImGui.TextDisabled(ctx, ("Last edit: %s"):format(state.edit.lastUndo))
  end
  ImGui.Separator(ctx)

  local items = state.items
  ImGui.Text(ctx, ("%d item(s) selected"):format(items.count))
  if items.truncated then
    ImGui.SameLine(ctx)
    ImGui.TextDisabled(ctx, "(too many to list)")
  end
  local flags = ImGui.TableFlags_Borders | ImGui.TableFlags_RowBg | ImGui.TableFlags_Resizable
    | ImGui.TableFlags_ScrollY
  if #items.entries > 0 and ImGui.BeginTable(ctx, "items", 5, flags, 0, 220) then
    ImGui.TableSetupScrollFreeze(ctx, 0, 1)
    ImGui.TableSetupColumn(ctx, "Track")
    ImGui.TableSetupColumn(ctx, "Take")
    ImGui.TableSetupColumn(ctx, "Position")
    ImGui.TableSetupColumn(ctx, "Length")
    ImGui.TableSetupColumn(ctx, "File")
    ImGui.TableHeadersRow(ctx)
    for _, item in ipairs(items.entries) do
      ImGui.TableNextRow(ctx)
      ImGui.TableNextColumn(ctx); ImGui.Text(ctx, item.track or "?")
      ImGui.TableNextColumn(ctx); ImGui.Text(ctx, item.name or "(no take)")
      ImGui.TableNextColumn(ctx); ImGui.Text(ctx, ("%.3fs"):format(item.position))
      ImGui.TableNextColumn(ctx); ImGui.Text(ctx, ("%.3fs"):format(item.length))
      ImGui.TableNextColumn(ctx); ImGui.Text(ctx, file_name(item.path))
      if item.path and ImGui.IsItemHovered(ctx) then ImGui.SetTooltip(ctx, item.path) end
    end
    ImGui.EndTable(ctx)
  end

  ImGui.Separator(ctx)
  local names = {}
  for _, track in ipairs(state.tracks.entries) do
    names[#names + 1] = ("%d: %s"):format(track.number, track.name ~= "" and track.name or "(unnamed)")
  end
  ImGui.Text(ctx, ("%d track(s) selected"):format(state.tracks.count))
  if #names > 0 then ImGui.TextWrapped(ctx, table.concat(names, ", ")) end
end

local ctx = ImGui.CreateContext("ReaSock Inspector")

local function loop()
  poll()
  ImGui.SetNextWindowSize(ctx, 560, 380, ImGui.Cond_FirstUseEver)
  local visible, open = ImGui.Begin(ctx, "ReaSock Inspector", true)
  if visible then
    draw(ctx)
    ImGui.End(ctx)
  end
  if open then reaper.defer(loop) end
end

reaper.defer(loop)
