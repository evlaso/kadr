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

## Spike results

Not run yet (phase 2).
