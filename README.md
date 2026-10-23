# kadr

Screenshots and screen recording, in Rust on GPUI CE. Made for Linux on [Omarchy](https://omarchy.org) (Arch, Hyprland, Wayland). Dark and light themes are built in and follow the system by default. English and Russian.

**Status:** works on Linux (Wayland, Hyprland); macOS and Windows build in CI but haven't been tried on a real machine, and recording and window picking aren't ported there yet. No release yet.

[Читать по-русски](docs/README.ru.md)

## What it does

- **One bar for everything** (`kadr capture`): area, fullscreen, window, scrolling, self-timer, text recognition and recording behind a single hotkey.
- **Screenshots** of an area, a window or the screen, and long scrolling ones (`kadr shot scroll`: pick an area and scroll, kadr stitches it). After a capture a thumbnail shows in the corner: copy, edit, pin on top, recognize text, drag the file into a chat. A click without selecting copies the color under the pointer. A window comes with rounded corners and a soft shadow on a transparent background. The selection can be kept and adjusted: handles, arrow keys, an aspect ratio (A) or a typed size; P or `kadr shot previous` takes the last area again.
- **Screenshot editor:**
  - arrows (bendable) and lines, rectangles and ellipses (outlined or filled), pencil, highlighter;
  - text (plain, on a plate or outlined), step numbers, pixelation, spotlight, crop;
  - a background in a sidebar (gradient, solid colour, wallpaper, the screenshot blurred or transparent), padding, rounding, shadow and an output format;
  - Save, Save as…, a `.kadr` project, Copy, pin on top and text recognition from the bottom bar;
  - presets: keep a look with “+”, pick, rename or delete it from a dropdown (the studio has the same);
  - pictures on top: drop a file into the editor or paste with Ctrl+V;
  - one size slider for every tool, zoomable canvas.
- **Screen recording:**
  - a bar by the selected area: microphone (with a level meter), system audio, camera, logged shortcuts, a countdown; the bar appears as soon as the mouse is let go, and the area stays adjustable there (handles, dragging, arrow keys, a size from the size menu or typed), and a take can be started over from the recording controls;
  - a camera bubble you set up before recording: drag it (it snaps to corners and edges), resize it, round it;
  - studio mode: the system cursor stays out of the video, kadr draws its own, large and smooth; the camera is recorded separately.
- **Studio** (video editor): settings by section in a rail at the left, a timeline with a zoom lane over the filmstrip, export settings under the Export button:
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
  - a tray icon;
  - project files: “Project” in the editor or the studio saves a `.kadr` file with the whole edit (the original picture or recording, every shape, zoom and cut), to open again later or on another computer. kadr registers the file type on its first run (`kadr install-desktop` does it by hand), so `.kadr` files open in kadr from the file manager.

## Install

kadr needs `gpu-screen-recorder`, `ffmpeg`, `tesseract` (with English data), `grim` and `wl-clipboard`. On Omarchy they're either there already or come from `pacman`. Building needs Rust 1.95 or newer.

```sh
cargo build --profile dist
install -m755 target/dist/kadr target/dist/kadr-tray ~/.local/bin/
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
kadr shot [smart|region|windows|fullscreen|timer|previous]
                                                   screenshot (previous: the last area again)
kadr shot scroll                                   scrolling screenshot
kadr record                                        recording bar (again to stop)
kadr record region|fullscreen [--studio] …         start recording at once
kadr record previous                               recording bar for the last area
kadr edit <image|project.kadr>                     screenshot editor
kadr studio <video|project.kadr>                   studio
kadr open <project.kadr>                           a project, in the editor or the studio
kadr text [image]                                  recognize text
kadr history                                       history
kadr settings                                      settings
kadr tray                                          tray icon
kadr install-desktop                               .kadr file type, icon and menu entry
```

`kadr --help` lists all of them.

## Platforms

- **Linux** (Wayland, Hyprland): everything works.
- **macOS and Windows:** they build in CI; not yet tried on a real machine. Screenshots, the clipboard, notifications, the tray icon and global hotkeys are written (through cross-platform crates); recording, picking windows and the studio's click log aren't ported yet. Everything system-specific lives in `crates/kadr-platform`, so it can be ported one function at a time.

Linux builds are attached to every commit on `main` in [Actions](https://github.com/vlasmn/kadr/actions); macOS and Windows ones to `v*` tags and manual runs.

## Development

How the code is laid out, the conventions and the known pitfalls are in [CLAUDE.md](CLAUDE.md).

## License

[MIT](LICENSE). Bundled third-party pieces keep their own licenses: the Lucide icons (ISC, `data/icons/ui/LICENSE-lucide.txt`), the fonts (SIL OFL 1.1, `data/fonts`), and the patched GPUI CE Linux platform crate (Apache-2.0, `vendor/gpui_linux`).
