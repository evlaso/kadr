# Install

Use a release build, or build kadr from source with Rust 1.95 or newer. Linux on Wayland with Hyprland is the main platform.

## Downloads

[Releases](https://github.com/evlaso/kadr/releases) has a Linux `.tar.xz` with `INSTALL.md`, a macOS `.dmg` containing `Kadr.app` for Apple silicon or Intel, and a Windows installer or portable `.exe`.

The builds are not signed yet. After copying the macOS app to Applications, run:

```sh
xattr -dr com.apple.quarantine /Applications/Kadr.app
```

## Linux (Hyprland)

Install `gpu-screen-recorder`, `ffmpeg`, `tesseract` with English language data, `grim` and `wl-clipboard`. On Omarchy they may already be installed; otherwise use `pacman`. `mpv` is optional: the trim window's Play and the studio's Preview play through it.

To build from source and install the two programs:

```sh
cargo build --profile dist -p kadr -p kadr-tray
install -m755 target/dist/kadr target/dist/kadr-tray ~/.local/bin/
```

Copy the Hyprland hook from the source tree:

```sh
cp data/hypr/kadr.lua ~/.config/hypr/kadr.lua
```

Add `require("hypr.kadr")` to `~/.config/hypr/hyprland.lua`. The hook floats kadr's windows, loads the shortcuts saved in [Settings](settings.md), and logs clicks and shortcut keys while recording for the studio. Linux starts with your existing Hyprland bindings, not the global defaults used on the other systems.

Start `kadr-tray` at login. In Omarchy's `autostart.lua`, add:

```lua
o.launch_on_start("kadr-tray")
```

On its first run, kadr registers the `.kadr` project file type, icon and menu entry on Linux. Run `kadr install-desktop` to do it by hand.

## macOS and Windows

For a source build, build all three programs and keep `kadr`, `kadr-tray` and `kadr-overlay` side by side:

```sh
cargo build --profile dist -p kadr -p kadr-tray -p kadr-overlay
```

The tray holds the global shortcuts, so start `kadr-tray` at login. The defaults and how to change them are in [Settings](settings.md). The Windows installer sets up the tray at login and the `.kadr` file association; the portable build is a single `kadr.exe`.

Unsigned builds are also attached to manually started [Actions](https://github.com/evlaso/kadr/actions) runs. For those standalone macOS programs, remove quarantine before opening them:

```sh
xattr -d com.apple.quarantine kadr kadr-tray kadr-overlay
```

Screenshots, window shots, the editor, studio and recording through the system's capture are implemented on macOS and Windows. Scrolling screenshots are available there too. These systems have CI tests, but have not yet been tried on a real machine. The Hyprland click/key log is Linux-only.

## Updating

With **General → Check for updates** on in Settings, the tray checks once a day. A new release appears on an update card. **Release page** opens its notes and downloads; **Update** downloads the release, verifies its release signature and replaces the installed files. The tray restarts on the new version. A version that fails to start three times in a row is replaced by the previous one. Package-manager installations are updated through their manager instead.

Choose **Update channel** in **General**: **Stable** gets finished versions only; **Beta** also gets alpha, beta and release-candidate versions. A pre-release build uses Beta until you change it.

To check immediately, open **Settings → About kadr → Check now**. The same row shows the result and offers the update. `kadr update` shows the result on a card instead.

## Commands

```text
  kadr settings                                  settings
  kadr update                                    look for a newer release now
  kadr tray                                      tray icon (for autostart)
  kadr install-desktop                           let .kadr project files open in kadr (file type, icon, menu entry)
```

`kadr install-desktop` is Linux-only. See [Command line](cli.md) for all commands.
