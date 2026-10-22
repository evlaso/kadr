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
- **Build**: `touch src/ui.rs`, then `cargo build --release`.

## Spike results

Not run yet (phase 2).
