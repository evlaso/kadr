-- kadr: while a recording runs, kadr creates $XDG_RUNTIME_DIR/kadr-input.log
-- and this hook appends clicks and key activity to it (nothing is written
-- otherwise). Install as ~/.config/hypr/kadr.lua and require("hypr.kadr").
--
--   c X Y        left click at the cursor
--   k            some key was pressed (anonymous; for auto-zoom)
--   x CODE p|r   with "show keys" on: a modifier changing state, or a key
--                pressed as part of a shortcut or a special key (Enter, Esc,
--                arrows…). Plain typing never gets a code, so text and
--                passwords typed during a recording are not logged.
local runtime = os.getenv("XDG_RUNTIME_DIR") or "/tmp"
local log_path = runtime .. "/kadr-input.log"
local keys_flag = runtime .. "/kadr-keys.on"

local function append(line)
  -- "r+" fails when the file doesn't exist, so this never creates it.
  local f = io.open(log_path, "r+")
  if not f then
    return false
  end
  f:seek("end")
  f:write(line, "\n")
  f:close()
  return true
end

local function exists(path)
  local f = io.open(path, "r")
  if f then
    f:close()
    return true
  end
  return false
end

hl.bind("mouse:272", function()
  local pos = hl.get_cursor_pos()
  append(string.format("c %s %s", pos and pos.x or "", pos and pos.y or ""))
end, { non_consuming = true, description = "kadr: log clicks while recording" })

-- evdev codes.
local MODIFIERS = { [29] = true, [97] = true, [42] = true, [54] = true, [56] = true, [100] = true, [125] = true, [126] = true }
local COMBO_MODS = { [29] = true, [97] = true, [56] = true, [100] = true, [125] = true, [126] = true }
local SPECIAL = {
  [1] = true, [14] = true, [15] = true, [28] = true, [96] = true, [111] = true, [110] = true,
  [102] = true, [107] = true, [104] = true, [109] = true, [103] = true, [105] = true, [106] = true, [108] = true,
  [59] = true, [60] = true, [61] = true, [62] = true, [63] = true, [64] = true, [65] = true, [66] = true,
  [67] = true, [68] = true, [87] = true, [88] = true,
}
local held = {}
local probed = false

local function field(ev, names)
  for _, name in ipairs(names) do
    local ok, v = pcall(function() return ev[name] end)
    if ok and v ~= nil then
      return v
    end
  end
end

hl.on("input.keyboard.key", function(ev, a2)
  if not exists(log_path) then
    return
  end
  local code, state
  if type(ev) == "table" or type(ev) == "userdata" then
    code = field(ev, { "keycode", "key", "code" })
    state = field(ev, { "state", "pressed", "down" })
  else
    code, state = ev, a2
  end
  local pressed = state == true or state == 1 or state == "pressed" or state == "down"

  if not exists(keys_flag) then
    if pressed or state == nil then
      append("k")
    end
    return
  end
  if not probed then
    -- What the event looks like (types only), to check the hook against
    -- this Hyprland version.
    probed = true
    append(string.format("? %s code=%s state=%s", type(ev), type(code), type(state)))
  end
  code = tonumber(code)
  if not code then
    return append("k")
  end
  if MODIFIERS[code] then
    held[code] = pressed or nil
    return append(string.format("x %d %s", code, pressed and "p" or "r"))
  end
  if not pressed then
    return
  end
  local combo = false
  for mod in pairs(held) do
    if COMBO_MODS[mod] then
      combo = true
    end
  end
  if combo or SPECIAL[code] then
    append(string.format("x %d p", code))
  else
    append("k")
  end
end)
