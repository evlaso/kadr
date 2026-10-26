## Platforms

| | Status |
|---|---|
| **Linux** (Wayland, Hyprland) | Everything works; used every day. |
| **macOS** (Apple silicon and Intel) | The editor, the studio, screenshots, the area picker, the tray and hotkeys pass the end-to-end test on CI. Not tried on a real Mac yet; no recording or window picking. |
| **Windows** (x86_64) | As on macOS. |

## Files

- `kadr-{version}-x86_64-linux.tar.xz`: `kadr`, `kadr-tray` and the Hyprland hook. Needs `gpu-screen-recorder`, `ffmpeg`, `tesseract`, `grim` and `wl-clipboard`; `INSTALL.md` inside says where things go.
- `kadr-{version}-macos-universal.dmg`: `Kadr.app`, for Apple silicon and Intel. Drag it to Applications; it lives in the menu bar.
- `kadr-{version}-x86_64-windows.zip`: portable, nothing to install. Start `kadr-tray.exe`.
- A `.sha256` beside each file: `sha256sum -c <file>.sha256` (on macOS `shasum -a 256 -c`).

## macOS: the first start

The app isn't signed or notarized yet, so macOS says it's damaged or from an unidentified developer. After copying `Kadr.app` to Applications, take it out of quarantine once:

```sh
xattr -dr com.apple.quarantine /Applications/Kadr.app
```

Then grant Screen Recording (System Settings → Privacy & Security) when it asks. Windows may show SmartScreen's warning for the same reason: More info → Run anyway.

## Testers wanted

kadr on macOS and Windows has only run on CI's machines. If you try it there, please [open an issue](https://github.com/evlaso/kadr/issues/new) with your system and version and what worked and what didn't; a screenshot helps. Reports from other Linux desktops are welcome too.

По-русски: [что нового](https://github.com/evlaso/kadr/blob/v{version}/docs/CHANGELOG.ru.md) и [README](https://github.com/evlaso/kadr/blob/v{version}/docs/README.ru.md).
