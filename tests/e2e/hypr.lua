-- The nested Hyprland the e2e tests and the bench run kadr in
-- (scripts/nested.sh): one 1920×1080 output at scale 1, nothing animated or
-- blurred, so captures and timings repeat.

hl.monitor({ output = "WAYLAND-1", mode = "1920x1080@60", position = "0x0", scale = 1 })

hl.config({
  general = { gaps_in = 0, gaps_out = 0, border_size = 0 },
  decoration = { rounding = 0, shadow = { enabled = false }, blur = { enabled = false } },
  animations = { enabled = false },
  misc = {
    disable_hyprland_logo = true,
    disable_splash_rendering = true,
    force_default_wallpaper = 0,
    background_color = "rgb(303036)",
  },
  cursor = { no_hardware_cursors = true },
  debug = { disable_logs = false },
})

-- kadr's ordinary windows float in the middle, as data/hypr/kadr.lua has them
-- in a real session, so their own size is what's tested.
hl.window_rule({ match = { class = "^kadr-(editor|settings|studio|history)$" }, float = true, center = true })
