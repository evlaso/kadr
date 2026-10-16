# kadr

CleanShot X–style screenshots and screen recording in Rust on GPUI CE (the community fork of Zed's GPUI). The main target is Linux: Omarchy (Arch, Hyprland, Wayland). macOS and Windows build in CI, and their system pieces are still stubs (see Platforms).

The user talks in Russian. All UI text is Russian. Code, comments and commit messages are in English.

## Build, run, check

- `cargo build --release`, then `install -m755 target/release/kadr ~/.local/bin/kadr`. Hyprland bindings and the tray call `kadr` from `~/.local/bin`.
- Before installing, `pkill -x kadr`: a running instance takes every new command over its socket, so the old binary would keep serving. That also kills the tray icon (`kadr tray` is a `kadr` process too); restart it afterwards with `setsid kadr tray &>/dev/null &`.
- `cargo test --release` for the unit tests (editor raster, studio camera and layout, key hints).
- `kadr --help` lists the commands. To try something without hotkeys, run e.g. `kadr edit <png>`, `kadr studio <mp4>`, `kadr record area "X,Y WxH"`, `kadr settings`, `kadr open <png>` (card).
- You can't click. Check visuals with `grim -o <monitor>` (focused monitor: `hyprctl monitors -j`) and crop with `magick`. Send keys with `wtype -k …`. `hyprctl dispatch movecursor` works only while the user isn't moving the mouse. For hover states, temporarily force the state in code, take the screenshot, then revert.
- `cargo check --target` for macOS and Windows doesn't work here: GPUI's build scripts need Xcode or MSVC. `.github/workflows/build.yml` builds all three systems.

## Layout

- `main.rs`: CLI. Single instance over `platform::ipc`: later calls pass their args to the running process. Quits when idle. Anything that must outlive its windows holds `kit::hold`.
- `platform.rs` + `platform/{linux,macos,windows}.rs`: everything OS-specific (clipboard, notifications, files, monitors, IPC, overlay windows, screenshots, signals). Put new OS-dependent code there, not inline.
- `ui.rs`: kadr's own components (Button, IconButton, ButtonLike, Label, Icon, Switch, Tooltip, KeyBinding, ProgressBar, Divider), the theme, and color/quad helpers. Don't add Zed's `ui`/`theme` crates back: they are GPL, and these replaced them on purpose.
- `palette.rs`: kadr's own dark and light colours (Sumi Dark, and white with the same blue), picked in the settings or following the system (`platform::prefers_dark`, since GPUI reports the appearance late on Wayland). Control fills and separators are the text color at a low opacity.
- `kit.rs`: assets (icons and fonts are compiled in), `REM`/`SCALE`, `open_layer` overlays, `open_window` for ordinary windows (editor, studio, settings; they float by the rule in `data/hypr/kadr.lua`), `InputRegion`.
- Screens: `cards` (quick-access card), `editor` (screenshot annotation), `studio` + `studio_editor` (zooms, trim and cuts, cursor, export through ffmpeg/libplacebo), `record` + `panel` + `track` (recording through gpu-screen-recorder, click/key log from the Hyprland Lua hook `data/hypr/kadr.lua`), `trim`, `history`, `settings` + `settings_view`, `ocr` (tesseract), `pin`, `job` (ffmpeg progress toast), `tray` (`kadr tray`, a separate process with no GPUI).

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
- Unusual glyphs (✦, ▣) may have no fallback font in GPUI CE. Prefer words or icons.

## Conventions

- Look and feel: CleanShot X UX in a soft, clean Apple-like look (references: CleanShot, Raycast, Atlas for Mac). No glass. Separate groups with space and tone, not lines. The main action uses the accent color.
- Before changing the user's Hyprland config (`~/.config/hypr/*.lua`), back up the file (`.bak-<timestamp>`).
- Commits go to `main` (GitHub `vlasmn/kadr`, private). End commit messages with the `Co-Authored-By` trailer the session specifies.
