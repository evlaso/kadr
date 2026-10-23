# kadr

Screenshots and screen recording for the desktop: capture an area, a window or the screen, annotate it, record with your camera and clicks, and turn a recording into a polished video. Written in Rust on [GPUI CE](https://github.com/gpui-ce/gpui-ce). Made first for Linux on [Omarchy](https://omarchy.org) (Arch, Hyprland, Wayland), coming to macOS and Windows. English and Russian; dark and light themes follow the system.

![The editor: an arrow, a box and a caption on a screenshot, on a gradient backdrop](docs/images/editor.png)

**Status:** works on Linux (Wayland, Hyprland). macOS and Windows build and pass their end-to-end test in CI (the editor, typing, the system's menus, the area overlay), but nobody has used them on a real machine yet, and recording and window picking aren't ported there. The first pre-release is [0.3.0-alpha.1](https://github.com/vlasmn/kadr/releases); testers on macOS and Windows are very welcome.

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

[Releases](https://github.com/vlasmn/kadr/releases) has builds for the three systems: a `.tar.xz` for Linux (with an `INSTALL.md`), `Kadr.app` in a `.dmg` for macOS (Apple silicon and Intel) and a portable `.zip` for Windows. They aren't signed yet: on macOS run `xattr -dr com.apple.quarantine /Applications/Kadr.app` once after copying the app. Homebrew, winget and the AUR come later. kadr can also be built from source, which needs Rust 1.95 or newer.

### Linux (Hyprland)

kadr needs `gpu-screen-recorder`, `ffmpeg`, `tesseract` (with English data), `grim` and `wl-clipboard`. On Omarchy they're either there already or come from `pacman`.

```sh
cargo build --profile dist -p kadr -p kadr-tray
install -m755 target/dist/kadr target/dist/kadr-tray ~/.local/bin/
```

Add the Hyprland hook: it floats kadr's windows, loads the hotkeys set in kadr's settings, and logs clicks and keys for the studio.

```sh
cp data/hypr/kadr.lua ~/.config/hypr/kadr.lua
```

Then add `require("hypr.kadr")` to `~/.config/hypr/hyprland.lua`, and start the tray icon from `autostart.lua`:

```lua
o.launch_on_start("kadr-tray")
```

### macOS and Windows

Build kadr, its tray and its area overlay, and keep the three side by side:

```sh
cargo build --profile dist -p kadr -p kadr-tray -p kadr-overlay
```

The tray holds the global hotkeys there, so start `kadr-tray` at login. Unsigned builds of every commit are attached to the [Actions](https://github.com/vlasmn/kadr/actions) runs started by hand; on macOS such a build needs `xattr -d com.apple.quarantine kadr kadr-tray kadr-overlay` before it opens.

## Hotkeys

Set them in kadr's settings (Shortcuts). The defaults on macOS and Windows; on Linux kadr starts from your Hyprland bindings:

| Keys | Does |
|---|---|
| Ctrl+Shift+1 | Screenshot of an area |
| Ctrl+Shift+2 | Recording |
| Ctrl+Shift+3 | Recognize text |
| Ctrl+Shift+4 | History |
| Ctrl+Shift+5 | Settings |
| Ctrl+Shift+6 | Scrolling screenshot |

While picking an area: Space for windows only, A for an aspect ratio, digits for a size, arrows to nudge (Alt+arrows to resize), P for the last area, Enter to take it, Esc to cancel.

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
- **macOS and Windows:** the editor, the studio, screenshots, the clipboard, notifications, the tray, global hotkeys and the area overlay are written and pass the end-to-end test on CI's runners; recording, picking windows and the studio's click log aren't ported yet. Everything system-specific lives in `crates/kadr-platform`.

## Development

[CONTRIBUTING.md](CONTRIBUTING.md) has how to build and test. Where the project is heading is [docs/SPEC.ru.md](docs/SPEC.ru.md) (in Russian); the measurements are in [docs/perf.md](docs/perf.md), known debts and pitfalls in [docs/traps.md](docs/traps.md), and [CLAUDE.md](CLAUDE.md) is the guide for coding agents.

## License

[MIT](LICENSE). Bundled third-party pieces keep their own licenses: the Lucide icons (ISC, `data/icons/ui/LICENSE-lucide.txt`), the fonts (SIL OFL 1.1, `data/fonts`), the fallback cursors (Adwaita by the GNOME Project, CC BY-SA 3.0, `data/cursors/LICENSE`), and the patched GPUI CE Linux platform crate (Apache-2.0, `vendor/gpui_linux`).
