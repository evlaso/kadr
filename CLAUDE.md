# kadr

CleanShot X–style screenshots and screen recording in Rust on GPUI CE (the community fork of Zed's GPUI). The main target is Linux: Omarchy (Arch, Hyprland, Wayland). macOS and Windows build in CI and share one implementation of most system pieces through cross-platform crates (`platform/desktop.rs`: arboard, notify-rust, trash, xcap; `tray.rs`: tray-icon, global-hotkey); recording and window picking there are still stubs. Nobody has run them on a real machine yet.

Where it's heading: `docs/SPEC.ru.md` (a workspace with the UI as a replaceable layer, tests, budgets, a measured experiment on leaving GPUI). Phase 0 is done: the base numbers in `docs/perf.md`, golden tests of the rasteriser, dev hooks.

The user talks in Russian. Code, comments and commit messages are in English.

UI text goes through `i18n` by key: `t("editor.save")`, or `tr!("settings.freed_files", size, n)` for text with values (plain `{}` slots only). Keys are `<area>.<name>` (`common.*` for text used in several places); the text lives in `data/i18n/en.json` and `ru.json`, and a test checks both have the same keys and slots. Add every new string to both. English is the default language; the README is English, with the Russian one in `docs/README.ru.md`.

## Build, run, check

- `cargo build --release`, then `install -m755 target/release/kadr ~/.local/bin/kadr`. Hyprland bindings and the tray call `kadr` from `~/.local/bin`.
- Before installing, `pkill -x kadr`: a running instance takes every new command over its socket, so the old binary would keep serving. That also kills the tray icon (`kadr tray` is a `kadr` process too); restart it afterwards with `setsid kadr tray &>/dev/null &`.
- `just unit` for the unit tests, `just golden` for the editor's rasteriser against `tests/golden/cases` (a failure leaves `target/golden/<case>/{actual,diff}.png`; `UPDATE_GOLDEN=1 just golden` rewrites the expected pictures, and the commit must say "golden updated: <cases>"), `just test` for both. `just bench` measures the budgets of `docs/SPEC.ru.md` §2 on the live session (the picker and an editor flash up) against `docs/perf.json`; results and method in `docs/perf.md`.
- Dev hooks: start kadr with `KADR_DEV=1`, then `kadr dev windows|dump|find|click|drag|key|type|state|shot …` answers in JSON (`src/dev.rs`). Buttons and switches report themselves by their element id; `dev state` gives the editor's drawing, tool and undo depth. This drives the UI without clicking: prefer it to moving the pointer.
- `kadr --help` lists the commands. To try something without hotkeys, run e.g. `kadr edit <png>`, `kadr studio <mp4>`, `kadr record area "X,Y WxH"`, `kadr settings`, `kadr open <png>` (card).
- You can't click. Check visuals with `grim -o <monitor>` (focused monitor: `hyprctl monitors -j`) and crop with `magick`. Send keys with `wtype -k …`. Move the pointer with `hyprctl eval 'hl.dispatch(hl.dsp.cursor.move({x = X, y = Y}))'` (the Lua config has no `dispatch movecursor`); it only sticks while the user isn't moving the mouse. Close a window with `hyprctl eval 'hl.dispatch(hl.dsp.window.close({ window = "address:0x…" }))'`: `hyprctl dispatch closewindow` fails quietly, and a kadr process whose window is still open keeps taking later commands. For hover states, temporarily force the state in code, take the screenshot, then revert.
- `cargo check --target` for macOS and Windows doesn't work here: GPUI's build scripts need Xcode or MSVC. `.github/workflows/build.yml` builds all three systems.

## Layout

- `main.rs`: CLI. Single instance over `platform::ipc`: later calls pass their args to the running process. Quits when idle. Anything that must outlive its windows holds `kit::hold`.
- `platform.rs` + `platform/{linux,macos,windows}.rs`: everything OS-specific (clipboard, notifications, files, monitors, IPC, overlay windows, screenshots, signals, window controls: `platform::window_controls()` says whether a top bar leaves room for macOS's traffic lights, draws Windows-style caption buttons, a close button on floating Linux desktops and Hyprland (where kadr's windows float), or nothing on other tiling managers; `kit::window_controls`/`kit::drags_window` draw it). Put new OS-dependent code there, not inline.
- `vendor/gpui_linux`: GPUI CE's Linux platform crate, patched in through `[patch]` in `Cargo.toml`, with drawing tablet support (`wayland/tablet.rs`). What's changed and how to bump it: `vendor/README.md`.
- `ui.rs`: kadr's own components (Button, IconButton, ButtonLike, Label, Icon, Switch, Tooltip, KeyBinding, ProgressBar, Divider), the theme, and color/quad helpers. Don't add Zed's `ui`/`theme` crates back: they are GPL, and these replaced them on purpose.
- `cursors.rs` + `data/cursors/*.svg`: the macOS-like cursor shapes the studio draws (arrow, pointer, text, grab…). A studio recording logs the pointer's shape over time through `platform::CursorShapes` (Linux: grabs around the pointer with and without the hardware cursor, matched against the user's XCursor theme, whose pictures are then drawn too; macOS and Windows: not yet, kadr's own SVGs are drawn).
- `palette.rs`: kadr's own dark and light colours (Sumi Dark, and white with the same blue), picked in the settings or following the system (`platform::prefers_dark`, since GPUI reports the appearance late on Wayland). Control fills and separators are the text color at a low opacity.
- `kit.rs`: assets (icons and fonts are compiled in: icons in `data/icons/{ui,editor,settings}` are all Lucide (ISC, license in `ui`; 24 grid, stroke 1.8, drawn at 16), so take new ones from Lucide too; only `editor/counter` and `editor/pixelate` are kadr's own on the same grid; fonts in `data/fonts` are OFL. No assets from Zed), `REM`/`SCALE`, `open_layer` overlays, `open_window` for ordinary windows (editor, studio, settings; they float by the rule in `data/hypr/kadr.lua`), `InputRegion`.
- Screens: `cards` (quick-access card), `editor` (screenshot annotation), `studio` + `studio_editor` (zooms, trim and cuts, cursor, export through ffmpeg/libplacebo), `record` + `panel` + `track` (recording through gpu-screen-recorder, click/key log from the Hyprland Lua hook `data/hypr/kadr.lua`), `trim`, `history`, `settings` + `settings_view`, `ocr` (tesseract), `pin`, `job` (ffmpeg progress, shown as a card), `presets` (the saved-looks dropdown shared by the studio and the editor), `tray` (`kadr tray`, a separate process with no GPUI; on macOS and Windows it also holds the global hotkeys from `hotkeys.rs`, so it runs even with the icon off, started at login), `bundle` (`.kadr` project files: a plain tar with `manifest.json` first; `editor/project.rs` and `studio::save_bundle`/`from_bundle` fill and read them; unpacked into `cache_dir()/projects/<hash>` with an `.origin` file pointing back, so saving from that session rewrites the `.kadr`; also the Linux file type/desktop entry, `kadr install-desktop`, done once automatically).

## Traps already hit

- GPUI clips children to rectangles only. Rounded images round themselves (`img().rounded()`), and bars inside a rounded window round their own outer corners.
- After the pointer leaves a surface, GPUI keeps its last position, so hover sticks. Gate hover on `window.is_window_hovered()` (see `cards.rs`).
- Wayland layer windows can't be moved or resized after creation. Use full-monitor layers plus `window.set_input_region` for click-through.
- `WindowKind::AnchoredPopup` with `grab: true` has to be opened from a mouse-down handler.
- GPUI CE `.transitions(...)` keeps the window redrawing (about 4% idle CPU). Don't use it until upstream fixes it.
- GPUI CE colors are `palette::Hsla`: use `.alpha`/`.lightness`/`.saturation`, `ui::rgb_of` and `ui::from_hex`. Paint with `ui::fill`/`outline`/`quad`.
- ab_glyph's `PxScale` is the font's full height, not the em. Always go through `editor::em()` so measured and drawn text match GPUI's.
- libplacebo asserts on fractional crops. Keep studio crop sizes whole, even pixels.
- Sizes for kadr's own overlays multiply by `kit::SCALE`. Component sizes use `ui::rems_from_px`.
- A dropdown menu drawn with `deferred` hangs outside its button's box, so the button's `on_mouse_down_out` closes it before a click in the menu lands. Keep the menu's bounds with `ui::track_bounds` and skip clicks inside them (see `presets.rs`).
- Tooltips are measured and shifted so they sit centred under the pointer (`ui::Tooltip`); GPUI itself puts them at the pointer's corner.
- Unusual glyphs (✦, ▣) may have no fallback font in GPUI CE. Prefer words or icons. GPUI CE doesn't fall back to other fonts for missing glyphs at all: a script the UI font lacks (e.g. Chinese) shows as boxes.
- The UI uses the system font (`.SystemUIFont`). Text drawn onto screenshots uses the bundled IBM Plex (`kit::ANNOTATION_FONT`), on the canvas too, so it matches the editor's rasteriser.

## Conventions

- Nested corners are concentric: an inner element's radius is the outer radius less the inset (see `cards::CORNER`, `history::BUTTON`, `studio_editor::TILE_RADIUS`). Scrolling panes get `ui::scrollbar`.
- Look and feel: CleanShot X UX in a soft, clean Apple-like look (references: CleanShot, Raycast, Atlas for Mac). No glass. Separate groups with space and tone, not lines. The main action uses the accent color.
- Before changing the user's Hyprland config (`~/.config/hypr/*.lua`), back up the file (`.bak-<timestamp>`).
- Commits go to `main` (GitHub `vlasmn/kadr`, private). End commit messages with the `Co-Authored-By` trailer the session specifies.
