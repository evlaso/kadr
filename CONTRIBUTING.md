# Contributing to kadr

Thanks for looking. kadr is a Cargo workspace in Rust; the UI is drawn with
[GPUI CE](https://github.com/gpui-ce/gpui-ce). Where the project is heading,
and why it's laid out the way it is, is in [docs/SPEC.ru.md](docs/SPEC.ru.md)
(in Russian); [CLAUDE.md](CLAUDE.md) is the same knowledge written for
coding agents and is worth a read for people too.

## Building

Rust 1.95 or newer. On Linux, the libraries GPUI needs (on Arch they're
there with a desktop; on Debian and Ubuntu see `.github/workflows/build.yml`).

```sh
cargo build --release            # while working: incremental, seconds
cargo build --profile dist       # what's shipped: whole-program optimised, ~2.5 min
```

`target/release/kadr` is the program; `kadr-tray` is the tray icon (and, on
macOS and Windows, the global hotkeys), `kadr-overlay` the area picker on
macOS and Windows. A running kadr takes every later command over its
socket: stop it (`pkill -x kadr`) before trying a new build.

## Testing

[`just`](https://github.com/casey/just) runs them:

| Command | What |
|---|---|
| `just unit` | Unit tests of every crate; view-models are tested here, without a window |
| `just golden` | The rasteriser against `tests/golden/cases`; a failure leaves `target/golden/<case>/{actual,diff}.png`. `UPDATE_GOLDEN=1 just golden` rewrites the pictures, and the commit says `golden updated: <cases>` |
| `just deps` | The crates' dependency rule (GPUI only in `kadr-ui-gpui`, the OS only in `kadr-platform`) |
| `just test` | All three |
| `just e2e` | kadr end to end, in a nested Hyprland of its own (Linux, inside Hyprland) |
| `just bench` | The budgets of SPEC §2, against `docs/perf.json` |

macOS and Windows run `scripts/desktop_e2e.py` in CI (the `desktop-e2e`
workflow): the editor, typing, the system's menus and the area overlay with
a real pointer, screenshots kept as artifacts.

## Where things go

- Logic of a screen: a view-model in `crates/kadr-ui`, tested there; the
  GPUI side in `crates/kadr-ui-gpui` only draws and wires it.
- Anything that differs between systems: `crates/kadr-platform`.
- Drawing onto pictures: `crates/kadr-raster`, with a golden case.
- Text in the UI goes through `i18n` by key; add every new string to both
  `data/i18n/en.json` and `ru.json`.
- Icons are Lucide; fonts are OFL. Nothing GPL.

## Commits and pull requests

Conventional commits: `feat:`, `fix:`, `perf:`, `ui:`, `docs:`, `test:`,
`refactor:`, `chore:`, with what changed for a person in the subject. The
changelog is drafted from them. Keep a pull request to one change, with its
tests; screenshots for anything you can see.

## Reporting

Bugs and ideas: [issues](https://github.com/evlaso/kadr/issues) (say your
system and kadr's version). Security problems: see [SECURITY.md](SECURITY.md).
