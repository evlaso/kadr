# kadr

Screenshots and screen recording in the spirit of CleanShot X, in Rust on GPUI CE. Made for Linux on [Omarchy](https://omarchy.org) (Arch, Hyprland, Wayland). Dark and light themes are built in and follow the system by default. English and Russian.

[Читать по-русски](docs/README.ru.md)

## What it does

- **One bar for everything** (`kadr capture`): area, fullscreen, window, scrolling, self-timer, text recognition and recording behind a single hotkey.
- **Screenshots** of an area, a window or the screen, and long scrolling ones (`kadr shot scroll`: pick an area and scroll, kadr stitches it). After a capture a thumbnail shows in the corner: copy, edit, pin on top, recognize text, drag the file into a chat. A click without selecting copies the color under the pointer.
- **Screenshot editor:**
  - arrows (bendable) and lines, rectangles and ellipses (outlined or filled), pencil, highlighter;
  - text (plain, on a plate or outlined), step numbers, pixelation, spotlight, crop;
  - a background (gradient, solid colour, wallpaper or transparent), padding, rounding, shadow and an output format;
  - presets: keep a look with “+”, pick, rename or delete it from a dropdown (the studio has the same);
  - pictures on top: drop a file into the editor or paste with Ctrl+V;
  - one size slider for every tool, zoomable canvas.
- **Screen recording:**
  - a bar by the selected area: microphone (with a level meter), system audio, camera, logged shortcuts, a countdown;
  - a camera bubble you set up before recording: drag it (it snaps to corners and edges), resize it, round it;
  - studio mode: the system cursor stays out of the video, kadr draws its own, large and smooth; the camera is recorded separately.
- **Studio** (video editor):
  - playback in the editor;
  - zooms on clicks, trimming the start and end, cutting pieces out;
  - background, padding, rounding, shadow, 16:9, 1:1, 4:5, 9:16;
  - click rings, shortcut badges, a cursor of any size and smoothness, motion blur;
  - the camera as a circle, rounded or square, in any of eight spots;
  - MP4 or GIF export with compression and a file-size estimate; the progress shows on a blurred thumbnail that turns into the result.
- **The rest:**
  - trimming a recording and making a GIF right from its thumbnail;
  - text recognition (English and Russian);
  - history of captures: search, filters, drag out;
  - settings with search, including hotkeys, theme, language, storage use and automatic clean-up of old captures;
  - a tray icon.

## Install

kadr needs `gpu-screen-recorder`, `ffmpeg`, `tesseract` (with English data), `grim` and `wl-clipboard`. On Omarchy they're either there already or come from `pacman`. Building needs Rust 1.95 or newer.

```sh
cargo build --release
install -m755 target/release/kadr ~/.local/bin/kadr
```

Add the Hyprland hook: it floats kadr's windows, loads the hotkeys set in kadr's settings, and logs clicks and keys for the studio.

```sh
cp data/hypr/kadr.lua ~/.config/hypr/kadr.lua
```

Then add `require("hypr.kadr")` to `~/.config/hypr/hyprland.lua`. Set hotkeys in kadr's settings (Shortcuts); start the tray icon from `autostart.lua`:

```lua
o.launch_on_start("kadr tray")
```

## Commands

```
kadr capture                                       every mode on one bar
kadr shot [smart|region|windows|fullscreen|timer]  screenshot
kadr shot scroll                                   scrolling screenshot
kadr record                                        recording bar (again to stop)
kadr record region|fullscreen [--studio] …         start recording at once
kadr edit <image>                                  screenshot editor
kadr studio <video>                                studio
kadr text [image]                                  recognize text
kadr history                                       history
kadr settings                                      settings
kadr tray                                          tray icon
```

`kadr --help` lists all of them.

## Platforms

- **Linux** (Wayland, Hyprland): everything works.
- **macOS and Windows:** they build in CI and the interface runs; capture, recording and the tray aren't ported yet. Everything system-specific lives in `src/platform/`, so it can be ported one function at a time.

Builds for all three systems are attached to every commit on `main` in [Actions](https://github.com/vlasmn/kadr/actions).

## Development

How the code is laid out, the conventions and the known pitfalls are in [CLAUDE.md](CLAUDE.md).
