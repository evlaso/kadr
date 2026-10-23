# Performance

The budgets of [SPEC.ru.md](SPEC.ru.md), section 2, measured by `just bench`
(`scripts/bench.py`). The base it compares against is `docs/perf.json`;
`just bench --save-base` replaces it.

## Base: GPUI CE, before any change to the UI (phase 0)

2026-10-23, the build with the dev hooks (commit after d22cea3), Linux,
Hyprland on the user's machine, median of 5 runs.

| Metric | Budget | GPUI CE |
|---|---|---|
| RSS of `kadr tray` after 5 min idle | ≤ 15 MB | 15.0 MB |
| RSS of the main process, editor with a 4K PNG | ≤ 250 MB | **388.6 MB** |
| Idle CPU of the main process, window open, no input, 30 s | ≤ 0.5 % | 0.1 % |
| Hotkey → overlay, warm | ≤ 80 ms | 33.4 ms |
| Hotkey → overlay, cold | ≤ 300 ms | 92.6 ms |
| `kadr edit <png>` → window with the picture | ≤ 400 ms | 297.3 ms |
| Release binary (stripped) | ≤ 25 MB | **44.7 MB** |
| Incremental `cargo build --release` (one UI file) | ≤ 20 s | 11.5 s |

Over budget: the editor's memory with a 4K picture and the binary's size.
The tray is right at its limit.

## Release profile: fat LTO, one codegen unit, abort on panic

2026-10-23, same machine and method.

| Metric | Base | Now |
|---|---|---|
| Release binary | 44.7 MB | **32.4 MB** (still over 25) |
| Editor with a 4K PNG, RSS | 388.6 MB | 351.1 MB (still over 250) |
| `kadr tray` after 5 min, RSS | 15.0 MB | 13.4 MB |
| Idle CPU | 0.1 % | 0.03 % |
| Hotkey → overlay, warm / cold | 33.4 / 92.6 ms | 34.7 / 90.4 ms |
| Edit → window | 297.3 ms | 299.7 ms |
| Incremental `cargo build --release` | 11.5 s | **146.5 s** (over 20) |
| Incremental `cargo build --profile fast` | — | 1.0 s |

The whole-program pass makes every release build a full one, against the
20 s budget for an incremental release build. Day-to-day builds and the
tests (`just`) use the `fast` profile instead: release optimisation without
LTO, incremental. The budget would need to name that profile, or LTO goes
into a separate distribution profile; the spec doesn't decide which.

Where the size goes (`cargo bloat --release --crates`, before the profile,
unstripped; .text 29.5 MiB):

| Crate | .text | Pulled in by |
|---|---|---|
| zbus | 3.7 MiB | ksni (tray), accesskit, oo7 (GPUI's keyring), notifications |
| gpui | 2.5 MiB | |
| std | 2.5 MiB | |
| image | 1.8 MiB | |
| ravif + rav1e | 2.2 MiB | AVIF encoding, from `image`'s default formats (GPUI) |
| naga + wgpu_core + wgpu_hal | 2.9 MiB | GPUI's renderer |
| gpui_ce_linux | 1.2 MiB | |
| kadr | 1.1 MiB | |
| accesskit_unix | 1.0 MiB | GPUI |
| exr, tiff, image_webp | 1.0 MiB | `image` formats kadr doesn't read |

The obvious cuts toward 25 MB: `image` without AVIF/EXR/TIFF (needs GPUI's
`image-default-formats` off), and fewer D-Bus stacks.

## Within the size budget, and phase 1 done

2026-10-23, after phase 1 (the crates, two copies of the picture) and the
size work below.

| Metric | Budget | Base | Now |
|---|---|---|---|
| `kadr tray` after 5 min, RSS | ≤ 15 MB | 15.0 MB | 12.3 MB |
| Editor with a 4K PNG, RSS | ≤ 250 MB | 388.6 MB | **234.8 MB** |
| Idle CPU, editor open | ≤ 0.5 % | 0.1 % | 0.07–0.13 % (one bench window caught 2.1 %, see below) |
| Hotkey → overlay, warm / cold | ≤ 80 / 300 ms | 33.4 / 92.6 ms | 33.6 / 88.5 ms |
| Edit → window | ≤ 400 ms | 297.3 ms | 294.1 ms |
| Release binary | ≤ 25 MB | 44.7 MB | **24.99 MB** |
| Incremental release build | ≤ 20 s | 11.5 s | 103 s (fat LTO; `fast`: 0.7 s) |

Every budget is met but the incremental release build (see the profile
section above). The idle CPU window that read 2.1 % didn't repeat (0.07 % and
0.13 % on two 30 s runs); the bench now takes the median of three windows.

What took the binary from 34.4 to 25.0 MB:
- GPUI's `image-default-formats` off: no AVIF encoder (rav1e, ravif), EXR or
  TIFF; kadr reads PNG, JPEG, WebP and PNM (and GPUI GIF). 34.4 → 26.4 MB.
- The Secret Service client (`oo7`) out of the vendored GPUI Linux crate,
  behind a `keyring` feature: kadr keeps no passwords.
- `opt-level = "s"` for crates off the hot path only: the D-Bus stack,
  accessibility, regex, the GIF and WebP decoders, SVG parsing, the shader
  translator (once at start), X11's protocol code, JSON and dates. The
  renderer, PNG, JPEG (the studio's scrub frames), text and the rasteriser
  keep full optimisation; cold start and edit → window didn't move.
  26.4 → 25.0 MB.
- The allocation counter only in `--features dev` builds.

The X11 backend costs about 14 KB, so it stays (best effort, per the spec).

## In a nested Hyprland: the base from now on

2026-10-23. The bench now runs kadr in a nested Hyprland of its own
(`scripts/nested.sh`: 1920×1080 at scale 1, its own runtime folder and D-Bus
bus), the same as the e2e tests, so the real session shows nothing and the
installed kadr and its tray are left alone. Numbers there differ a little
from the live session's, so `docs/perf.json` is this run's.

| Metric | Budget | Nested |
|---|---|---|
| `kadr tray` after 5 min, RSS | ≤ 15 MB | 11.5 MB |
| Editor with a 4K PNG, RSS | ≤ 250 MB | 233.7 MB |
| Idle CPU, editor open | ≤ 0.5 % | 0.0 % |
| Hotkey → overlay, warm / cold | ≤ 80 / 300 ms | 56.8 / 67.2 ms |
| Edit → window | ≤ 400 ms | 295.3 ms |
| Binary (dist) | ≤ 25 MB soft, 30 hard | 25.0 MB |
| Incremental `--release` build | ≤ 20 s | 0.8 s |

All within budget. The tray now waits for a panel that isn't there yet
(`assume_sni_available`), which the nested session needs (it has none) and
a login where kadr starts before the panel does too.

## Profiles: release for work, dist for shipping

2026-10-23, the user's decision: `--release` has no whole-program pass and
builds incrementally, so the 20 s incremental budget is measured on it
(0.75 s for a one-line change); `dist` (fat LTO, one codegen unit, the same
per-crate opt-levels) is what's shipped and what the size is measured on
(24.99 MB). The size budget is 25 MB soft (a warning) and 30 MB hard; no
squeezing beyond that.

## Copies of the picture in the editor

`kadr dev mem` with a 3840×2160 picture (a noise PNG of 40.8 MB, so the
file's bytes show; a real screenshot's PNG is a few MB):

| | Before | After |
|---|---|---|
| The file's bytes, kept for the project | 40.8 MB | — (encoded from the picture on save) |
| The file's bytes again, for GPUI's own image | 40.8 MB | — |
| GPUI's decoded copy of that image | 31.6 MB | — |
| The picture, decoded (RGBA) | 31.6 MB | 31.6 MB |
| The picture as the canvas shows it (BGRA, padded) | 31.7 MB | 31.7 MB |
| **Live blocks over 1 MB** | **178.5 MB** | **65.3 MB** |
| RSS (fast build with line tables and dev hooks) | 654 MB | 533 MB |

What's left is the picture once to draw and export from, and once on the
GPU's side; drawings are a layer of shapes on top, and undo keeps changes
to it, not copies.

## How each is measured

- **Tray RSS**: a fresh `kadr tray` (on Linux the StatusNotifierItem through
  ksni), `VmRSS` from `/proc` after the idle time.
- **Editor RSS and idle CPU**: `kadr edit` on a 3840×2160 noise PNG; RSS two
  seconds after the picture is in, then CPU ticks over 30 s without input.
- **Hotkey → overlay**: from starting `kadr shot region` (what the hotkey
  runs) until Hyprland lists the `kadr-picker` layer for that process. Cold:
  no kadr running. Warm: a kadr is already running (an editor open), the new
  call hands over through the socket. The layer being listed is a little
  before its first frame shows; the spec's log mark in the tray isn't there
  yet.
- **Edit → window**: from starting `kadr edit` until `kadr dev state` reports
  the decoded picture.
- **Build**: a one-line change in `src/ui.rs` (a touched file alone isn't
  rebuilt by an incremental profile), then `cargo build --release` and
  `cargo build --profile fast`.

## Spike results (phase 2)

2026-10-23. Each shell implements the settings, a card and the editor with
three tools (arrow, box, text) over the same view-models (`kadr-ui`) and
rasteriser (`kadr-raster`): GPUI CE (the app), iced 0.14.0 (branch
`spike/iced`, `crates/kadr-ui-iced`), Tauri 2.12.5 with the system's
WebKitGTK (branch `spike/tauri`, `crates/kadr-ui-tauri`). All measured by
`scripts/spike.py <shell>` in the nested Hyprland, the same way, median of 3;
memory and CPU over the whole process tree (Tauri's page draws in WebKit's
own processes).

| Metric | GPUI CE | iced | Tauri |
|---|---|---|---|
| Edit → window with a 4K picture | 304 ms | **179 ms** (−41 %) | 671 ms (+121 %) |
| Editor RSS with a 4K picture | 263 MB | 301 MB (+14 %) | 564 MB (+114 %, 3 processes) |
| Idle CPU, editor open | 0.1 % | 0.1 % | 0.0 % |
| Binary (dist) | 25.0 MB | **17.6 MB** | 7.6 MB, plus the system's WebKitGTK |
| Incremental `--release` build | 0.7 s | 0.4 s | 0.6 s |
| Arrow, box and text with a real pointer and keyboard, saved | yes | yes | yes |
| Settings search, a card that goes in time | yes | yes | yes |

Not compared: hotkey → overlay (the spec gives the overlay its own crate,
`kadr-overlay`, shared by every shell; not built yet), and the tray's idle
RSS (the tray has no UI framework, so it's the same for all).

The decision rule (SPEC §9.4), as far as it can be checked here:

1. No Linux metric more than 15 % worse than GPUI: **iced passes** (memory
   +14 %, everything else better or equal); **Tauri fails** (opening +121 %,
   memory +114 %).
2. The spike running on macOS or Windows without changes to the framework's
   platform layer: **not checked** — no Mac or Windows machine here, and the
   CI's macOS and Windows jobs don't start (the account's billing).
3. The UI tests passing unchanged: the spikes have no dev hooks; the same
   pointer-and-keyboard check (`scripts/spike.py`, `kadr-pointer`, `wtype`)
   ran unchanged against all three and passed, which shows the view-models
   are apart from the frameworks, but it isn't the §6.3 hook suite.
4. The editor with a 4K picture not lagging while dragging: a person's call.

So: Tauri is out on the numbers. iced is a candidate that hasn't failed
anything measurable, but rules 2 and 4 need the user (a Mac or Windows run,
and a hand on the editor) before a migration is decided. Until then kadr
stays on GPUI CE and both spike branches are kept.
