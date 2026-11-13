# kadr: technical specification of the target version

Status: in force. The document is in English, like identifiers and code.
Purpose: describe an architecture in which the UI framework is a replaceable layer, cover the
project with automated tests and record the decisions taken.

**Framework decision (2026-10-23): kadr stays on GPUI CE.** The spike (section 9) showed that iced
gives no gain on Linux, Tauri loses twice over on memory and start-up, and GPUI CE fairly passed
desktop-e2e on macOS and Windows (on Windows, on WARP with no graphics card). The split into crates
stays as insurance: if GPUI CE stops being developed, only `kadr-ui-gpui` is rewritten.

Where things stand: phases 0 and 1 (section 9) are done, phase 2's spike is over, and phase 3 won't happen
unless the question comes up again. Where this document describes a target the code hasn't reached, a
"Today" note says what there is now.

## 0. Project identifiers

| What | Value |
|---|---|
| Product name | kadr |
| Licence | MIT, `Copyright (c) 2026 Evgeniy Vlasov` |
| Repository | `github.com/evlaso/kadr` (public); if it moves, the `kadr-sh` organisation |
| Domain | `kadr.sh`; the site is GitHub Pages; releases and the updater are `https://kadr.sh/releases` → GitHub Releases |
| Bundle id | `sh.kadr.app` (macOS), `sh.kadr.Kadr` (desktop entry / Flatpak), the same root in winget |
| AUR | `kadr-sh` (from the tarball), `kadr-sh-bin`, `kadr-sh-git`; `provides=('kadr') conflicts=('kadr')`: the name `kadr` is taken by another project, and the binary `/usr/bin/kadr` conflicts |
| crates.io | not published (the name `kadr` is squatted; the GUI is distributed as binaries) |

---

## 1. Goals and principles

1. **Cross-platform without compromises.** Linux (Wayland/Hyprland is the main one, X11 best effort),
   macOS, Windows. One code base, the same behaviour, platform differences only in `kadr-platform`.
2. **Resources.** When idle, one small process with no UI framework lives on. Everything heavy starts
   on demand and is shut down. The budgets are in section 2 and are checked automatically.
3. **The UI is a replaceable layer.** No crate except `kadr-ui-*` imports the UI framework's types.
   Everything that can be checked without a window is checked without a window.
4. **Determinism.** The annotation rasteriser, video export and the bundle are reproducible byte for byte
   (or within a threshold for the rasteriser); otherwise golden tests are impossible.
5. **AI writes the code, a person accepts it.** Hence: one command for all checks, machine-readable
   results (JSON, diff pictures), test hooks in the app itself, documented traps.

## 2. Budgets (checked by `just bench`, which fails when one is exceeded)

| Metric | Target | How to measure |
|---|---|---|
| RSS of `kadr tray` after 5 min idle | ≤ 15 MB | `/proc/<pid>/status` VmRSS; mac `ps -o rss`; win `Get-Process` |
| RSS of the main process, the editor with a 4K PNG | ≤ 250 MB | the same |
| Idle CPU of the main process with a window open, no input, 30 s | ≤ 0.5 % | `pidstat`/`top -b` |
| Hotkey → overlay visible, process alive (warm) | ≤ 80 ms | a log mark in the tray at the hotkey → `dev shot` catches the first frame with the overlay |
| Hotkey → overlay visible, cold start | ≤ 300 ms | the same |
| `kadr edit <png>` → window with the picture | ≤ 400 ms | log marks |
| Size of the `dist` profile's binary (stripped) | ≤ 25 MB a warning, ≤ 30 MB an error | `stat` |
| `cargo build --release` incremental (1 UI file) | ≤ 20 s | `time` |

Profiles: `--release` has no fat LTO and is incremental, for development and the build budget;
`dist` has `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, for releases and the size budget.
Don't squeeze the size budget at the cost of features: 25 MB is a guide, not a goal.

The base and the history of measurements are in `docs/perf.md`. The bench runs only in a nested Hyprland (6.4);
in a live session the numbers can't be trusted.

## 3. Architecture: a Cargo workspace

```
kadr/
  Cargo.toml              # workspace
  crates/
    kadr-core/            # data model and logic, 0 dependencies on the OS and the UI
    kadr-raster/          # annotation rasteriser: tiny-skia, ab_glyph
    kadr-platform/        # everything OS-specific: linux/macos/windows + desktop.rs; the single-instance socket (ipc)
    kadr-media/           # recordings and the studio without a UI: the edit, the export's ffmpeg graph, Job
    kadr-tray/            # daemon: tray + global hotkeys, no UI framework
    kadr-overlay/         # area picker overlay, drawn on the CPU (see 3.8)
    kadr-ui/              # screens as state (view-models), design tokens, no framework
    kadr-ui-gpui/         # the UI on GPUI CE, the only crate that names GPUI
    kadr-testkit/         # tools for the e2e tests: kadr-pointer, kadr-shot
    kadr-mcp/             # planned: MCP server (section 11)
  src/main.rs             # the `kadr` binary: CLI
  tests/                  # e2e (tests/e2e.rs, tests/e2e/hypr.lua) and golden cases
  scripts/                # bench, the nested session, desktop e2e, release packaging, the manual, check-deps
  docs/                   # SPEC.md (this file), ROADMAP.md, perf.md, traps.md, manual/
  data/                   # i18n, icons, fonts, cursors, the Hyprland hook (hypr/kadr.lua), app icons
  vendor/                 # GPUI CE's Linux platform crate, patched (vendor/README.md)
  website/                # the manual's site (12.5)
```

There are no `kadr-ocr` and `kadr-ipc` crates: text recognition (tesseract) is the app's (`kadr-ui-gpui`'s `ocr.rs`),
the socket is `kadr_platform::ipc` and the command line `kadr_core::cli` (3.5).

The dependency rule (checked by `scripts/check-deps.sh`, `just deps`): `core ← raster ← ui`, `core ← platform`,
`core, raster ← media, overlay`, `core, platform ← tray`; `kadr-ui-gpui` sits on top of them all, and the binary uses
`core`, `platform`, `tray` and `ui-gpui`. An arrow backwards is forbidden.
`gpui`, `tauri`, `iced` and the like appear only in `kadr-ui-*`; the script checks the dependencies and the code.

### 3.1 `kadr-core`

- `geometry`: `Rect`, `Point`, `Size` in logical and physical pixels with an explicit `Scale`;
  snapping to window/monitor edges; normalising the selection.
- `document`: the editor's annotation model. Versioned JSON (`"version": N`), migrations between
  versions. Types: `Arrow`, `Rect`, `Ellipse`, `Line`, `Pen`, `Text`, `Counter`, `Blur`, `Pixelate`,
  `Highlight`, `Crop`, `Redact`. Each is plain data (coordinates, style, z-order), with no drawing methods.
- `history`: undo/redo as a list of commands over `Document`; the invariants are checked by tests.
- `settings`: typed settings + serialisation, defaults, migrations.
- `presets`: saved "looks" (shared by the editor and the studio).
- `bundle`: the `.kadr` format (tar, `manifest.json` first). Writing/reading, a round-trip test.
- `i18n`: keys, `t()`/`tr!()`, a test that `en.json`/`ru.json` have the same keys and slots.
- `storage`: paths (cache, config, data), file naming, the history of shots (an index).

Today: `kadr-core` has `document` (the shape kinds are `Arrow`, `Rect`, `Ellipse`, `Line`, `Pen`, `Marker`, `Text`,
`Counter`, `Cursor`, `Blur` (pixelate, blur or a black box), `Spotlight`, `Picture`; the crop is the document's own,
not a shape), `color`, `project` (the drawing as `project.json`, no version field yet), `bundle`, `settings`, `i18n`,
`storage`, `dirs`, `hotkeys`, `cli` and `update`. Undo is `kadr_ui::history`; the saved looks are the app's (`presets.rs`)
and the studio's (`kadr_media::studio`); `geometry` isn't a module of its own yet.

### 3.2 `kadr-raster`

The only way to get a picture from a `Document`:
`render(base: &RgbaImage, doc: &Doc, frame: &Frame) -> Option<RgbaImage>` (and `render_png`, `render_as` for a format).

- Deterministic: fixed fonts (bundled IBM Plex), no system fonts, no time.
- The editor uses the same code for the canvas preview and for export: they can't differ, by construction.
- Text metrics: only through `raster::text::em()`, so that measured and drawn text match.
- Scale (1×, 2×) is a parameter, not a global.

### 3.3 `kadr-platform`

```rust
pub trait Platform: Send + Sync {
    fn monitors(&self) -> Vec<Monitor>;             // id, bounds, scale, name, primary
    fn windows(&self) -> Vec<WindowInfo>;           // for the window picker
    fn capture(&self, target: CaptureTarget) -> Result<Capture>;  // Screen|Window|Region
    fn clipboard(&self) -> &dyn Clipboard;          // image + text + file
    fn notify(&self, n: Notification) -> Result<()>;
    fn trash(&self, path: &Path) -> Result<()>;
    fn open_with_default(&self, path: &Path) -> Result<()>;
    fn prefers_dark(&self) -> bool;
    fn cursor_shapes(&self) -> Option<Box<dyn CursorShapes>>;  // for the studio
    fn autostart(&self, enabled: bool) -> Result<()>;
    fn permissions(&self) -> Permissions;           // macOS: screen recording, accessibility
    fn window_controls(&self) -> WindowControls;    // traffic lights / caption buttons / none
}
```

- `linux`: xdg-desktop-portal (`ashpd`) as the main capture path; what's specific to Hyprland (hyprctl,
  layer-shell, the Lua hook for clicks/keys) is behind the `hyprland` feature; X11 through `xcap`.
- `macos`: ScreenCaptureKit through `objc2-*`; checking and requesting permissions; `NSStatusItem` in the tray crate.
- `windows`: Windows.Graphics.Capture through the `windows` crate (fallback `xcap`).
- **Cursors in the studio** (`CursorShapes`): on every OS the system's own are taken while recording: Linux from the
  user's theme (XCursor), macOS through `NSCursor`, Windows through `GetCursorInfo`/`DrawIconEx`. The repository
  holds only one fallback set, `data/cursors/*.png`: the Adwaita cursors (GNOME, CC BY-SA 3.0, unchanged, hotspots
  in `kadr_raster::cursors::BUNDLED`), for systems with no theme and for the golden tests. GPL cursor sets and
  other systems' own pictures aren't bundled.
- Everything that doesn't depend on the OS but depends on "the desktop" (arboard, notify-rust, trash) is in `desktop.rs`.
- For tests: `FakePlatform` with given monitors/windows and capture from PNG fixtures.

Today there's no `Platform` trait and no `FakePlatform`: each OS has a module with the same free functions
(`linux.rs`, `macos.rs`, `windows.rs`, picked at build time; `desktop.rs` shared by macOS and Windows, with
screenshots through `xcap`), beside `dialog`, `keymap`, `menu`, `record` (the recorders), `style`, `update` and `ipc`.
Linux takes the screen through `grim` and a window by itself through ext-image-copy-capture (`toplevel.rs`);
there's no `hyprland` feature.

### 3.4 `kadr-media`

- The studio's model: the timeline, zooms, cuts, trim, the cursor (path + shapes), export presets: plain data.
- `layout`: working out the frame (camera, crop): pure functions, covered by tests (already there).
- Runners: wrappers over `ffmpeg`/`libplacebo`/`gpu-screen-recorder` as a `Job` with progress over a channel.
  Commands are built by pure functions (`fn ffmpeg_args(&Export) -> Vec<String>`) and tested
  as strings; the processes themselves are behind a `Runner` trait, with a `FakeRunner` in tests.
- libplacebo's invariants (whole, even crops) live in the types (`EvenSize`), not in comments.

Today: `kadr-media` has the edit (`studio::Project`, `Camera`, `layout`), the export's ffmpeg graph, a recording's log
(`track::Track`), the camera picture's look (`camera`) and ffmpeg with progress (`job::Job`). The recorders
(gpu-screen-recorder on Linux, ScreenCaptureKit on macOS, Windows.Graphics.Capture on Windows) are
`kadr_platform::record`. There's no `Runner` trait and no `EvenSize` yet.

### 3.5 `kadr-ipc`

There's no such crate: the socket is `kadr_platform::ipc`, the command line is parsed by `kadr_core::cli`, and the
hooks are `kadr-ui-gpui`'s `dev.rs`.

- One socket (a Unix socket; on Windows, a localhost port), JSON lines: a call sends its arguments as one JSON array.
  The first process is the server; later calls to `kadr …` pass their arguments on and exit.
- The public commands are the CLI (`kadr --help`, `cli::USAGE`): `capture`, `shot …`, `record …`, `edit <image>`,
  `studio <video>`, `open <file>`, `pin <file>`, `trim <video>`, `gif <video>`, `text [image]`, `history`, `settings`,
  `update`, `tray`, `dev …`, `install-desktop`.
- **Dev hooks** (only with `KADR_DEV=1`; a `--features dev` build adds `dev mem`'s list of allocations), section 6.3.

### 3.6 `kadr-tray`

- A separate binary/process with no UI framework: the icon (`tray-icon` / `ksni`), the menu, global hotkeys
  (`global-hotkey`; on Hyprland, through the compositor's bindings), autostart.
- On a hotkey: if the main process is alive, an IPC command; otherwise, spawn `kadr <cmd>`.
  It writes the mark `hotkey_at=<monotonic ms>` to its log for the latency bench.
- Whether it keeps the main process "warm" is a setting (`keep_warm: bool`, off by default on Linux,
  on for macOS/Windows if the cold start doesn't fit the budget). Today it's on by default on macOS only.
- Today: the binary is `kadr-tray` (`kadr tray` too, for old autostart entries). The icon is `ksni` on Linux and
  `tray-icon` elsewhere; the global hotkeys (`global-hotkey`) are on macOS and Windows, and on Linux the
  compositor's bindings call `kadr`.

### 3.7 `kadr-ui` (no framework)

```rust
pub trait Shell {
    fn open_editor(&self, session: EditorSession) -> WindowHandle;
    fn open_studio(&self, session: StudioSession) -> WindowHandle;
    fn open_settings(&self) -> WindowHandle;
    fn show_card(&self, card: Card) -> WindowHandle;        // quick-access card
    fn show_panel(&self, panel: RecordPanel) -> WindowHandle;
    fn pin(&self, image: PinImage) -> WindowHandle;
    fn run(self, ipc: IpcServer) -> !;                      // event loop, quit on idle
}
```

- The view-models (`EditorVm`, `StudioVm`, `SettingsVm`, `CardVm`) live here: state, commands,
  undo, the chosen tool, hit-testing over the `Document`. The UI framework only draws and sends events.
  This lets the editor's behaviour be tested without a window (section 6.1) and makes replacing the framework
  a rewrite of the "view", not of the logic.
- The theme and palette (`Sumi Dark` / light), sizes (`REM`, `SCALE`), icons (Lucide, bundled) are here,
  as data. Drawing the components is in `kadr-ui-*`.

Today there's no `Shell` trait, and the view-models are `EditorVm` (with undo, `history`, which keeps changes,
not copies), `SettingsVm` and `CardsVm<T>`; there's no `StudioVm` yet. Keys come in as `kadr_ui::Key`. Beside
them, the components' logic without a framework: `text::Field` (a typed text: caret, selection, the input method's
composing text), `popover` (placement and dismissal), `controls` (slider mapping and stepping). The shared values
are `kadr_ui::tokens`: sizes, radii, spacing, type, alphas, tones, shadows, base colours and the shared components'
metrics, each with its unit, bounds and drag step; `tokens::store` reads and saves a restyle (`tokens.toml` beside
`settings.toml`, only the differences from the defaults). The palette, `REM`/`SCALE` and the icons are still
the app's (`palette.rs`, `kit.rs`).

### 3.8 `kadr-overlay`

The area picker overlay is a separate crate drawn on the CPU (tiny-skia, no GPU context): on Wayland a
layer-shell surface through `smithay-client-toolkit`, on a thread of kadr; on macOS and Windows a window
through `winit` + `softbuffer`, in the `kadr-overlay` program:
the dimming, the frame, the sizes, the magnifier, snapping to windows, key hints. The reasons:

- latency: no GPU/webview start-up, the window and its first frame in tens of milliseconds;
- independence from the UI framework: one overlay for all the spike's candidates;
- on Linux without Vulkan the overlay still works.

On Wayland, a `layer-shell` over the whole monitor + an input region for click-through; on macOS, an `NSPanel`
at the `screenSaver` level that doesn't activate; on Windows, a layered topmost window. If during the spike
a candidate framework turns out to draw the overlay no worse by the budget, the crate stays as a fallback.
Outcome: kadr stayed on GPUI CE, `kadr-overlay` is the picker, and GPUI's own picker is the fallback.

### 3.9 `kadr-ui-gpui`

The implementation of `Shell`. Only drawing, events, windows. No business logic.
The rule "not a single gpui type outside this crate" stays in force and is checked by `scripts/check-deps.sh`:
it's the insurance in case GPUI CE stops being developed.

With no `Shell` trait today, the crate is the app's UI as it is: every screen, its windows and events. Its
components are kadr's own (Zed's GPL `ui`/`theme` crates aren't used): `ui.rs` (buttons, labels, icons, switches,
tooltips, key hints, the theme), `ui::text_field` (single-line fields over `kadr_ui::text::Field`, through GPUI's
input handler), `ui::popover` (elevated panels, menu rows, dropdowns) and `ui::controls` (option chips,
checkboxes, sliders, swatches, the backdrop picker). They read their sizes from `kadr_ui::tokens`.
The component gallery (`kadr gallery`, left out of `--help`) shows every shared component in each variant
and state, drawn by the real code, with the theme and language switchable for the run; its Sizes page edits
the tokens live, restyling every open window, and saves a restyle to `tokens.toml`.

### 3.10 Platform edges: feeling native without native widgets

The UI is drawn by kadr itself and is the same on every OS (as in apps that draw their whole UI themselves). The native feel
comes from the edges, where the app meets the system. These are requirements, not wishes:

- **The system's dialogs** for open/save/choose a folder: only `rfd`, none of kadr's own.
- **Native menus**: on macOS (the menu bar and context menus) and Windows, through `muda`; on Linux/Wayland
  there are no system menus outside GTK/Qt, so kadr draws the menus there.
- **Keyboard conventions**: a table in `kadr-platform::keymap`: Cmd/Ctrl, `Cmd+,` for settings,
  `Cmd+W`/`Cmd+Q` on macOS, `Alt+F4`/`Ctrl+,` on Windows; `Home/End` vs `Cmd+←/→`,
  `Option+←/→` by words. Key hints (`KeyBinding`) draw the OS's symbols (⌘⌥⇧ / Ctrl Alt Shift).
- **Text input**: input methods, the system's emoji picker, selecting by words: checked in desktop-e2e
  on macOS and Windows.
- **Platform style constants** in `kadr-platform::style`: the window radius (macOS 10–12, Windows 11 8,
  Linux by the compositor), the kind of toggle (Switch on macOS, Switch everywhere by default), the order of buttons
  in dialogs (confirm on the right on macOS, on the left on Windows), window controls (already there).
- **Window behaviour**: the editor and the settings shrink to fit the screen and are centred at any resolution
  (a trap from the spike: a 1360×760 window on a 1024×768 screen went past the edge); a test in desktop-e2e.
- **The system font**, the dark theme following the system, the native tray and notifications: already there, don't break them.
- **Accessibility** (screen reader): a known debt of a UI of our own on GPUI CE; recorded in `docs/traps.md`,
  doesn't block a release.

## 4. Process model

```
[kadr tray]  — always; ≤15 MB; hotkeys, icon, autostart
     │ IPC / spawn
[kadr]       — on demand; Shell + overlay; quits N s after no windows and no jobs
     │ spawn
[ffmpeg | gpu-screen-recorder | tesseract] — separate processes, a Job with progress
```

- Logs: `tracing` to the file `cache_dir()/kadr.log` + `--log-json` for the benches. Today there's neither:
  kadr writes to stderr (with `KADR_DEV=1`, GPUI's notes and warnings too), a crash leaves `cache_dir()/crash.log`,
  and the tray notes its hotkeys in `cache_dir()/kadr-tray.log`.
- A crash of the main process doesn't touch the tray and doesn't lose a recording (the recorder writes straight to a file,
  the studio's manifest is saved incrementally).

## 5. Data contracts

- `Document` JSON: the schema in `docs/schema/document.v<N>.json`, generated from the types (`schemars`);
  a test checks that fixtures of every version can be read. Today the drawing is `project.json`
  (`kadr_core::project`), with no version field and no schema yet; `docs/schema/` doesn't exist.
- The `.kadr` bundle: `manifest.json` (`version`, `kind: shot|studio`, `app`, `files`), then the files.
- `settings.toml` (before 0.4, `settings.json`, moved over on its own): a version + migrations, also edited by hand, a person's comments are kept; a test "settings v1 are read by v<N>".
- IPC: `{"cmd": "...", "args": {...}, "id": n}` → `{"id": n, "ok": true, "result": ...}`. Today a call is one line,
  its arguments as a JSON array; `kadr dev …` waits for a JSON line back with `ok`.

## 6. Automated tests

Running everything: `just test` (= `just unit && just golden && just ui && just e2e`). Today `just test` is
`just deps`, `just unit` and `just golden`; `just e2e` and `just bench` run on their own, and there's no `just ui`
or `just lint` yet. Each layer is its own
command, so that the AI runs the layer it needs quickly. What each failure leaves is a path to an artifact
(a diff png, a JSON dump, a log), which the agent opens itself.

### 6.1 Unit (`just unit` = `cargo test --workspace --exclude kadr-ui-*`)

Today `just unit` is `cargo test --release --workspace -- --skip golden`; `proptest` and `insta` aren't used yet.

- `kadr-core`: geometry/snapping (property tests with `proptest`: normalising is idempotent, snapping doesn't go
  outside the monitor), undo/redo (after `n` do + `n` undo the document equals the original), bundle round-trip,
  schema migrations, i18n keys, presets.
- `kadr-ui` view-models: scenarios like "picked the arrow → drag → the document holds an Arrow with these coordinates",
  "hit-test lands on the topmost by z", "Esc drops the tool", "Ctrl+Z after text removes the text".
  No window, no framework: this is the main way to test the editor's behaviour.
- `kadr-media`: `ffmpeg_args` by snapshot (`insta`), the camera layout, the even-crop invariants.
- `kadr-platform`: parsers (`hyprctl -j`, `xdg` answers) on fixtures; `FakePlatform`.
- Target time: `just unit` ≤ 60 s.

### 6.2 Rasteriser golden tests (`just golden` = `cargo test -p kadr-raster --test golden`)

- `tests/golden/cases/<name>/{base.png, project.json, expected.png}` (a project as `.kadr` keeps it), written from
  `cases()` in `crates/kadr-raster/tests/golden.rs` by `just golden-cases`; one case per tool + combinations
  (text on a blur, an arrow across a crop, counter 1–99, 2× scale).
- The comparison is perceptual with a threshold (SSIM of the luma ≥ 0.995, computed in the test), plus a limit
  on the number of pixels with |Δ| > 8 (0.1 % of the picture). Byte for byte only for `Blur`/`Pixelate`
  (determinism is a must there).
- On a failure it writes `target/golden/<name>/{actual.png, diff.png}`. The expected pictures are updated
  only with `UPDATE_GOLDEN=1 just golden`, and the commit must mention "golden updated: <cases>":
  that's the signal for a person to look at the diff.
- Fonts: bundled only; the test fails if the rasteriser reached for a system font.

### 6.3 Dev hooks and UI tests without a compositor (`just ui`)

Hooks in the app (on with `KADR_DEV=1`), available over IPC and the CLI `kadr dev …`:

| Command | Result |
|---|---|
| `dev windows` | the list of windows: id, kind, bounds, scale |
| `dev dump [window]` | the element tree: `test_id`, bounds, text, state (hovered/pressed/disabled/checked), scroll |
| `dev find <test_id>` | one element |
| `dev click <test_id> [--button] [--at dx,dy]` | a synthetic click in the element's centre |
| `dev hover <test_id>` / `dev unhover` | a synthetic hover (no pointer) |
| `dev key <combo>` / `dev type <text>` | input |
| `dev drag <id> <dx,dy>` | a drag from an element |
| `dev shot <path> [window]` | a screenshot of the window by the framework's means (or the compositor's) |
| `dev state` | the JSON of the current window's view-model (document, tool, undo depth) |
| `dev set-theme dark\|light`, `dev set-scale 1\|2` | for determinism |

- Every interactive element gets a `test_id` (`editor.tool.arrow`, `settings.hotkeys.area`).
  A lint test: every `Button`/`IconButton`/`Switch` in `kadr-ui-*` is created with a `test_id`.
- The tests in `tests/ui/*.rs` start `kadr --features dev` with `FakePlatform`, work through the hooks and
  check `dev state`/`dev dump`. No screenshots are compared here: only structure and state.
- If GPUI CE keeps `test-support` (`TestAppContext`), some of these tests can run in-process
  without a window; that's an optimisation, the contract is the hooks.

Today (`crates/kadr-ui-gpui/src/dev.rs`): the hooks are `windows`, `dump`, `find`, `click`, `drag`, `scroll`, `drop`,
`key`, `type`, `state`, `shot`, `token` (read or restyle a token for the run), `grab-window` and `mem`; `hover`/`unhover`,
`set-theme` and `set-scale` aren't there yet. Buttons and switches report themselves by their GPUI element id
(`theme-Dark`, `section-<name>`); there's no lint test for the ids. There's no `tests/ui/` and no `just ui`: the tests
that work through the hooks are in `tests/e2e.rs` and run in the nested Hyprland (6.4).

### 6.4 E2E on a compositor (`just e2e`, Linux; macOS/Windows: desktop-e2e in CI, section 7)

- `scripts/nested.sh` starts a nested Hyprland with a test config (`tests/e2e/hypr.lua`):
  one 1920×1080 output @1×, no animations, no blur, no gaps or rounding, no X server (Xwayland off),
  its own runtime folder, D-Bus session bus and kadr folders, `kadr` from `target/release`.
  Its window sits in the real session, floating on a workspace that's never shown (`name:kadr-e2e`), and keeps
  rendering there. A headless output would cut the tie to the real screen, but aquamarine can't allocate its
  buffers on the development machine (GBM on NVIDIA, `docs/traps.md`), so `nested.sh start` checks that frames come.
  `nested.sh start` prints the variables (`WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE`, the `XDG_*` folders,
  `KADR_DEV=1`) for the tests. A real pointer there is `kadr-testkit`'s `kadr-pointer` (wlr-virtual-pointer).
- Scenarios (Rust tests in `tests/e2e.rs`, each marked `Scenario N`, among others for the editor, the studio and
  the shared components):
  1. the tray is running → a hotkey (through `hyprctl dispatch`) → the overlay is visible within the budget → drag → a PNG in the history,
     its size and content as from `grim -g` of the same area (SSIM ≥ 0.99).
  2. `kadr edit fixture.png` → `dev click editor.tool.arrow` → `dev drag` → `dev key ctrl+s` →
     the file equals the golden rasteriser's output for the same document (linking the layers 6.2 and 6.4).
  3. `kadr record area …` 3 s → `kadr studio` → export → `ffprobe` gives the expected size/duration.
  4. settings: switch the theme → `dev shot` → compare with the expected picture, with a mask over the system font.
  5. single instance: a second `kadr edit` reuses the process (one PID).
- Screenshots of screens are compared perceptually, as in 6.2; the expected pictures are kept at 1× and 2×.
- Today: scenario 1 runs the hotkey's command (`kadr shot region`) rather than the key, and compares the shot with
  the screen grabbed before; scenario 3 covers the studio's half only (gpu-screen-recorder takes the real screen,
  not the nested one); scenario 4 compares brightness, since the system font differs by machine. No expected
  screenshots are kept for e2e.

### 6.5 Benches (`just bench`)

- `scripts/bench.py` takes the metrics of section 2 into `target/bench.json`, compares them with `docs/perf.json`
  (the base; `--save-base` replaces it, the history is in `docs/perf.md`) and the budgets; fails when a budget is
  exceeded, warns on a regression > 15 % from the base.
- It runs in the e2e environment (the same nested Hyprland), 5 runs each, the median.

### 6.6 Static checks (`just lint`)

`cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny` (licences: GPL forbidden, Apache/MIT/ISC/OFL fine),
`scripts/check-deps.sh` (the dependency graph from section 3), the i18n key test, the `test_id` test.

Today there's no `just lint`: `just deps` runs `scripts/check-deps.sh` and the i18n key test is part of `just unit`;
fmt, clippy and `cargo deny` aren't wired in.

## 7. CI

- The repository is public, so Actions minutes are free. GitHub Actions, a matrix of `ubuntu` (unit, golden,
  lint, e2e, bench), `macos`, `windows` (unit, golden, lint, build `dist`).
- `desktop-e2e` (macOS + Windows): opens the editor on the runner's desktop, picks a tool with a real pointer,
  draws, saves; checks that the whole window is on the screen and that the tool really
  is picked (the button's colour through `kadr-shot pixel`). `result.json`: `outcome` ∈ `passed | failed |
  gpu_unavailable`, `renderer`, `gpu` (the adapter; WARP is recognised by "Basic Render Driver"), the coordinates of the window and
  the clicks. `gpu_unavailable` is not the framework's failure but a reason for a run on a real machine. It runs on
  `workflow_dispatch` and on a tag.
- Artifacts on a failure: `target/golden/**`, `target/e2e/**` (shots, dumps, logs), `target/bench.json`,
  `desktop-e2e-<os>` (screenshots of the steps, `app.log`, `result.json`).
- A PR with no changes in `kadr-ui-gpui` doesn't run e2e (path filter), for speed.
- A release on a `v*` tag: a `dist` build on three OSes, artifacts in GitHub Releases, updating the AUR package
  `kadr-sh-bin` and the Homebrew tap.
- Today (`.github/workflows`): `build.yml` builds `dist` and runs `cargo test --release` on Linux on every push
  and pull request, and on all three systems on a `v*` tag or by hand; `desktop-e2e.yml` runs
  `scripts/desktop_e2e.py gpui` on macOS and Windows on a tag or by hand; `release.yml` is 12.2; `docs.yml` publishes
  the site (12.5). Lint, e2e and the bench don't run in CI yet, and nothing updates the AUR or Homebrew.

## 8. Conventions (carried over from CLAUDE.md)

- i18n by keys, both locales, a test on the keys. Lucide icons, OFL fonts, nothing from Zed, no GPL.
- Concentric corners, space instead of lines, the accent on the main action. No "glass".
- New OS-specific code goes only into `kadr-platform`. New screen logic goes into `kadr-ui` (a view-model), not into the "view".
- `docs/traps.md` is the living list of the framework's traps; a migration starts a new section.
- Commits go to `main`, with the `Co-Authored-By` trailer from the session.

## 9. The migration experiment: finished

**Outcome (2026-10-23): kadr stays on GPUI CE.** The `spike/*` branches were deleted after the results were written
to `docs/perf.md`. Below is the protocol the decision was made by; it stays in case the question comes up
again (for example, if GPUI CE stops being developed).

The spike in short (Linux, nested Hyprland, the editor with a 4K picture):

| | GPUI CE | iced 0.14 | Tauri 2.12 |
|---|---|---|---|
| Opening the editor | 304 ms | 179 ms | 671 ms |
| The editor's memory | 263 MB | 301 MB (+14 %) | 564 MB (+114 %) |
| Idle CPU | 0.1 % | 0.1 % | 0 % |
| `dist` binary | 25.0 MB | 17.6 MB | 7.6 MB + the system's WebKit |
| desktop-e2e macOS / Windows | passed / passed (WARP) | passed / passed | — |

Tauri dropped out on the Linux metrics. iced is on a par, with no gain, and rewriting the editor and the studio
doesn't pay off. GPUI CE works on macOS and Windows, including a runner with no graphics card.

### 9.1 Phase 0: the base (before any UI changes)

1. Take the metrics of section 2 on the current build → `docs/perf.md`.
2. Golden tests of the rasteriser on the current `editor.rs` (before splitting out crates: this protects the refactoring).
3. Dev hooks at a minimum: `dump`, `click`, `key`, `state`, `shot`.

Done (2026-10-23): the base is in `docs/perf.md`, the golden tests in `crates/kadr-raster/tests/golden.rs`, the hooks in 6.3.

### 9.2 Phase 1: the split (without changing the framework)

Split `kadr-core`, `kadr-raster`, `kadr-platform`, `kadr-media`, `kadr-ipc`, `kadr-ui` (view-models)
out of the current code. The GPUI screens become `kadr-ui-gpui`. Done when: `scripts/check-deps.sh` is green,
`just unit && just golden` are green, behaviour hasn't changed (e2e scenarios 1–5 pass on GPUI).

Done (2026-10-23, `docs/perf.md`), with no `kadr-ipc`: the socket went into `kadr-platform` (3.5). Beside the
crates named here, the workspace has `kadr-tray`, `kadr-overlay` and `kadr-testkit` (section 3), and `kadr-ui`
has since taken the cards' and settings' state, the shared components' logic and the tokens.

### 9.3 Phase 2: the spike (time-box 2 evenings, branch `spike/<candidate>`)

Candidates: **Tauri 2** (the editor/settings in a webview; the overlay is `kadr-overlay`) and **iced** (a pinned
version). For each, implement `Shell` to this extent: `open_settings`, `show_card`, `open_editor` with three
tools (arrow, rectangle, text) over `EditorVm` and `kadr-raster`. No more.

Take the metrics of section 2 for each; build and run on macOS **and** Windows (at least one of the two
on a real machine). Fill in the table in `docs/perf.md`: rows gpui / tauri / iced.

Done: the table is in `docs/perf.md` and at the start of section 9; the decision was to stay on GPUI CE, and the
`spike/*` branches are gone.

### 9.4 The decision rule

Migrate to a candidate if all of these hold:

1. no metric on Linux is worse than GPUI's by more than 15 %, and hotkey→overlay and idle RSS aren't worse at all;
2. the spike started on macOS or Windows with no changes to the framework's platform layer;
3. the UI tests through the hooks (6.3) for the implemented screens pass with no changes to the tests themselves,
   i.e. the view-models really are separate;
4. subjectively: the editor with a 4K PNG doesn't lag while dragging (a person checks; one condition for all).

Otherwise kadr stays on GPUI CE, the spike is deleted, and the PR with the tablet patch goes upstream to gpui-ce.

### 9.5 Phase 3: migration (if decided)

Screen by screen: settings → cards → history → pin → panel → editor → studio. The old `kadr-ui-gpui`
stays buildable behind a feature until the end; switching is by `--features ui-<candidate>`. Each screen
is accepted when: `just ui` is green on both implementations, the screen's e2e scenario is green, the expected screenshots
are updated with an explicit review. Removing GPUI is the phase's last commit.

Not taken: the decision (9.4) was to stay on GPUI CE.

## 10. Definition of Done for the whole specification

- `just test` and `just bench` are green on Linux, locally and in CI; on macOS/Windows, unit/golden are green
  and `desktop-e2e` has the status `passed`.
- `docs/perf.md` holds the base and the spike's results; the decision from 9.4 is written there too, with the date.
- Any screen can be tested without a person clicking: through `kadr dev …`.
- Any PR that changes the rasteriser carries a diff of the golden pictures, checked by a person.
- The platform edges of 3.10 are implemented and covered by desktop-e2e.

## 11. Agent mode (MCP)

kadr is a "camera" for AI agents: the agent goes through the scenario itself (a browser, computer use, kadr's dev hooks),
and kadr records, annotates and edits. Scenarios: a screencast after a change to a prototype, a UI audit,
a test with video proof, onboarding, a tutorial. They differ only in the scenario, not in the tools.

Planned: milestone 0.6 in `docs/ROADMAP.md` (its section 8). None of this section is built yet.

### 11.1 `kadr-mcp`

A separate binary, `kadr mcp`, on `rmcp` (the official Rust SDK for MCP), stdio transport. A thin adapter
over kadr's socket (`kadr_platform::ipc`): each tool is one socket command; the core doesn't change. It is started from the config
of an MCP client (Claude Code, Claude Desktop and others).

Tools:

| Tool | What it does |
|---|---|
| `capture` (`screen` \| `window` \| `area`, `monitor?`) | a PNG; returned as image content and a path |
| `windows` / `monitors` | a list to choose the target from |
| `record.start` (`target`, `preset?`) → `recording_id` | starts recording; the click/key log is written as it is now |
| `record.marker` (`text`, `kind?: step \| expect \| note`) | a marker with a monotonic time and a text |
| `record.stop` → path to the `.kadr` | stops; the bundle holds `script.json` with the markers |
| `studio.export` (`bundle`, `preset`, `narration?: bool`, `subtitles?: bool`) → mp4 | edits by the markers |
| `ocr` (`png`) | the text |
| `dev.*` | only with `KADR_DEV=1`: the same hooks as in 6.3 |

Security: the MCP server works only locally, over stdio; the first `capture`/`record` call from a new
client needs confirming in kadr's UI (a card "Agent X asks for access to the screen", remembered in the settings).

### 11.2 The script as the source of truth

`script.json` in the `.kadr` bundle:

```json
{ "version": 1,
  "steps": [
    { "t": 0.0,  "kind": "step",   "text": "Open the settings", "narration": "Click the gear…" },
    { "t": 4.2,  "kind": "expect", "text": "The list of hotkeys is visible" }
  ] }
```

- The markers set the steps' timing → the studio's zooms and chapters (a zoom's frame comes from the clicks inside the step's interval).
- `narration` is voiced by TTS within the step's interval; the subtitles are the same text with the same timing.
  No speech recognition is needed, they can't drift apart, and editing the text edits the video without re-recording.
- The script is edited in the studio by hand (the "Script" tab) and by the agent (`studio.script.update`).

### 11.3 Voice-over and subtitles (`kadr-media`)

- TTS is a `Job` like ffmpeg; the engine is local by default (Piper or Kokoro), so that recordings of internal
  prototypes don't go to the cloud; a cloud TTS is an option in the settings.
- Subtitles: burned in (through ffmpeg) and/or a separate `.srt`/`.vtt`.
- Tests: `ffmpeg_args` by snapshot; synthesis is behind a `Tts` trait with a `FakeTts`, so that unit tests don't depend on models.

### 11.4 Order

After 3.10 and `kadr-overlay`: `record.marker` in the recording protocol and `script.json` in the bundle → `kadr-mcp` with
`capture`/`record.*`/`studio.export` → TTS and subtitles → confirming access in the UI.

## 12. Releases, updates, changelog

### 12.1 Versions

SemVer. While at `0.x`: minor for new features or a change to the `.kadr`/settings format (with a migration),
patch for fixes. The first public release is `0.3.0`; its pre-releases are tagged `v0.3.0-alpha.1` and
`v0.3.0-alpha.2`, and `Cargo.toml` says `0.3.0-alpha.2` (the history isn't reset).
`1.0.0` comes when all of these hold: macOS and Windows have been checked by real users (not only by runners),
the overlay is native on all three OSes, the `.kadr` format and `settings.toml` are frozen (they change only with a migration),
the self-update has survived at least two releases in a row. One version for the workspace (`[workspace.package] version`).

### 12.2 The release pipeline

- A `v<semver>` tag on `main` → the `release` job: a `dist` build on three OSes, the artifacts:
  Linux: `kadr-<ver>-x86_64-linux.tar.xz` (+ `aarch64`), AppImage later;
  macOS: `Kadr.app` in a `.dmg`, universal (arm64 + x86_64);
  Windows: `kadr-<ver>-x86_64-windows.msi` and a `.zip` (portable).
  The tool is `cargo-dist` for the build, the installers and the Homebrew tap; the `.app` bundle and the `.dmg` are a step of our own
  (`scripts/bundle-macos.sh`), until `cargo-dist` can do them.
- Signing: macOS, Developer ID + notarisation (Apple Developer, $99/year; without it Gatekeeper blocks the app
  and self-update is impossible; required for `1.0`, until then `xattr -d com.apple.quarantine` in the README);
  Windows, Azure Trusted Signing, or unsigned with a SmartScreen warning until `1.0`;
  every artifact gets a `minisign` signature with the project's key, the public key built into the binary.
- CI publishes a GitHub Release with the body from `CHANGELOG.md` and puts `latest.json` on the `gh-pages` branch
  → `https://kadr.sh/releases/latest.json`:
  ```json
  { "version": "0.3.0", "date": "2026-11-01", "notes_url": "https://kadr.sh/releases/0.3.0",
    "platforms": { "linux-x86_64": { "url": "...", "sha256": "...", "sig": "..." },
                   "macos-universal": { ... }, "windows-x86_64": { ... } } }
  ```
- After a release CI updates `kadr-sh-bin` in the AUR (an ssh key in the secrets) and the Homebrew tap; winget gets a PR to
  `microsoft/winget-pkgs` through `wingetcreate`.

Today (`.github/workflows/release.yml`, with no `cargo-dist`): Linux `kadr-<ver>-x86_64-linux.tar.xz`
(`scripts/package.sh`); macOS `kadr-<ver>-macos-universal.dmg` (`scripts/bundle-macos.sh`, unsigned);
Windows `kadr-<ver>-x86_64-windows-setup.exe` (Inno Setup, for the user only) and a portable
`kadr-<ver>-x86_64-windows-portable.exe` (`scripts/package.sh`, unsigned). Each file gets a `.sha256` and,
with the release key in the secrets, a `minisign` signature; the release body is the version's CHANGELOG section
followed by `.github/release-notes.md`; `scripts/release.py` writes
`releases/latest.json` on `gh-pages`, with `channels`, and its `notes_url` is the GitHub release. Until the
domain is up, kadr reads it from the `gh-pages` branch on GitHub (`kadr_core::update::LATEST_URL`). Nothing goes to
the AUR, Homebrew or winget yet.

### 12.3 Self-update (`kadr-update`, a crate in `kadr-platform` or its own)

- **The check**: `kadr-tray` asks for `latest.json` once a day and at start (no more than once an hour);
  compares semver; the result goes to `cache_dir()/update.json`. It can be turned off in the settings. No telemetry:
  the request is a plain GET with no identifiers.
- **Channels**: Stable, only versions with no pre-release suffix; Beta, also `-alpha`/`-beta`/`-rc`,
  and the stable one if it's newer than the latest beta. By default, the installed build's channel
  (a pre-release → Beta); it can be changed in the settings. `latest.json` holds `channels.stable` and
  `channels.beta`; the top level is the newest release, for clients from before the channels. On going from
  Beta to Stable the version doesn't go down: kadr waits for a stable one newer than the installed one.
- **The install source** is found at start and changes the behaviour:
  `self` (an archive/dmg/msi from the site) → full self-update;
  `aur` / `brew` / `winget` / `flatpak` → only a notice "0.4.0 is available: update through <manager>"
  (distributions and Homebrew don't put up with self-updating binaries).
  Detection: Linux, the binary's path (`/usr/bin` → a package, `~/.local/bin` → self) and a marker file from the PKGBUILD;
  macOS, `brew --prefix` in the path; Windows, a marker from winget/msi. Today (`kadr_platform::update::install`):
  Linux, under `/usr` → the package manager found there, else self; macOS, a Homebrew path → Homebrew, else
  the `Kadr.app` it runs from; Windows, the installer's uninstaller beside the exe → installed, else portable;
  a build under `target/` is left alone.
- **The flow**: a card in the tray/settings "Version 0.4.0 is available · What's new · Update".
  Pressing it → a background download into `cache_dir()/updates/`, checking the sha256 and the minisign signature → replacing:
  Linux, an atomic replacement of the `kadr` and `kadr-tray` binaries (`self-replace`);
  macOS, replacing `Kadr.app` beside it (renaming, not copying inside), which needs signing;
  Windows, the downloaded installer (Inno Setup) runs silently (`/VERYSILENT`) and kadr exits;
  a portable `kadr.exe` is renamed and swapped.
  The tray restarts at once; kadr itself moves to the new version once its windows close
  (it exits when idle anyway).
- A "quietly in the background" mode (download ahead, apply at the next start) is a setting, off by default.
- Rollback: the previous files are kept beside the new ones (`kadr.prev`, `kadr-tray.prev`; `.Kadr.app.prev` on macOS;
  `kadr.exe.prev` for the portable one) until the new version settles after a start; if it fails to settle three
  times (`kadr_core::update::TRIES`), kadr puts the old files back at its next start (`cache_dir()/updates/pending.json`
  keeps the count).
- Tests: `latest.json` is parsed from fixtures; the update flow is an e2e test with a local HTTP server
  and a fake release, checking the signature, the replacement and the rollback.

Today the check and the release's data (channels, versions, the signature, what's pending) are `kadr_core::update`;
how kadr was installed, the download and the replacement are `kadr_platform::update`; the card and Settings → About's
Check now are the app's `update.rs`. The background mode isn't there yet.

### 12.4 Changelog

- `CHANGELOG.md` in the Keep a Changelog format, written by hand, with no generator: the conventional commits
  since the last release (`feat:`, `fix:`, `perf:`, `ui:`) are the draft, **rewritten in the user's language** before
  the tag: three to seven points "what changed for a person", no crate names and no internal refactorings;
  a screenshot or a GIF for big features. In English only: `CHANGELOG.md`.
- The GitHub Release body is the version's section of the CHANGELOG (CI cuts it out automatically).
- **The What's new screen**: at the first start of a new version, a card with the changelog
  section of that version; the markdown is drawn by kadr's own components (heading, list,
  picture). Also reachable from the settings. The text comes from the app's bundle, not from the network:
  it works offline and doesn't depend on GitHub. The changelog is English only, like kadr's other documents.
  Today there's no such screen: the update card ("kadr <ver> is out") says that what's new and the downloads are on
  the release page, links to it (its body is the version's CHANGELOG section) and has Update where kadr updates
  itself; Settings → About's Check now does the same in its row.

### 12.5 Open-source repository hygiene

- `README.md`, in English: what it is, a screenshot/GIF of the current interface, installing
  on three OSes (brew / winget / AUR / download), the platforms' status, hotkeys, a link to the spec and CONTRIBUTING.
- `CONTRIBUTING.md`: how to build, `just test`, conventional commits, what lives where (a link to the spec).
- The user manual `docs/manual` (a page per screen, all of the same shape, in English only, plus install, project files,
  the command line, languages and the glossary; `crates/kadr-core/tests/manual.rs` keeps the pages' shape, the command
  and settings pages true to the code, and the languages page's share of translated strings for each file of `data/i18n`)
  is the site on GitHub Pages (`scripts/manual.sh`, Eleventy from `website`, published by `docs.yml`): there's no
  separate landing page, the manual's first page is the home, with the download from `releases/latest.json`. The site
  also draws a roadmap page from `docs/ROADMAP.md` and a changelog page from `CHANGELOG.md`, and writes `llms.txt`,
  `llms-full.txt` and a Markdown copy of every page for language models. A feature's commit changes its screen's page.
- `CLAUDE.md` stays as the guide for agent contributors (many projects call it `AGENTS.md`;
  you can keep one and symlink the other), with no personal paths or local settings: those go in `.claude/settings.local.json`
  and in `.gitignore`.
- `SECURITY.md` (where to report vulnerabilities: `security@kadr.sh`), issue templates (a bug with the OS and version,
  a feature), `.github/FUNDING.yml` if wanted.
- The repository's description: "Screenshots and screen recording for Linux, macOS and Windows. Native,
  written in Rust." Other people's trademarks aren't mentioned in public texts. Topics: `screenshot`, `screen-recording`, `rust`, `gpui`, `wayland`, `hyprland`,
  `cross-platform`.
- Cleaning up: `vendor/` stays until the upstream PR is accepted; remove from the repository everything not needed for building
  and testing (spike artifacts, temporary scripts, personal Hyprland configs: only the templates in `data/hypr` stay).
