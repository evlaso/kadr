# Changelog

What changed in each kadr release, for the people who use it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). In Russian: [docs/CHANGELOG.ru.md](docs/CHANGELOG.ru.md).

## [Unreleased]

### Added

- Arrow styles in the editor: a filled head, an open head, a head at both ends, or tapered from a thin tail into the head. Any of them bends, and a selected arrow takes another style.
- Hiding is harder to undo: the blur first brings the area down to blocks with a little noise and then smooths it, so it looks as soft as before but no deblurring brings the text back, and each block of a pixelation is nudged by noise, so it can't be matched against pixelated guesses.
- A black box among the hide tool's choices (B): what's under it is gone from the saved file and from the copy kept to edit it again, so removing the box later shows black. Hide private info covers with black boxes too when they're picked.
- Colours in the editor: the pipette (I) takes one from the picture, the colour menu keeps the ones picked lately, and + keeps the current colour among your own (right-click one to remove it). Both lists live in `settings.toml`.
- The editor opens JPEG and WebP pictures, and Save As writes PNG, JPEG or WebP by the name you give it (JPEG on white where the picture is transparent; WebP without loss). Saving a JPEG opened in the editor keeps it a JPEG.
- While recording, the bar shows the microphone filling with how loud it is, so you can see you're heard.
- Updating from kadr itself: the update card's Update button downloads the release, checks it against the release key's signature and puts it in place of the installed files, and the tray starts again on it. On Windows the installer runs quietly; the portable kadr.exe swaps itself. A version that fails to start three times in a row is replaced by the previous one. Packages from pacman or Homebrew are left to their manager.
- Update channels in Settings → General: Stable gets finished versions only, Beta gets alpha, beta and release-candidate versions as well. A pre-release build is on Beta until you change it.
- Check now in Settings → About, which says right there whether there's a newer version, and updates from the same row; `kadr update` on the command line shows it on a card.
- Scrolling screenshots on macOS and Windows, and sideways: scroll the area to the right (a timeline, a wide table) and kadr stitches it into one wide picture.
- Settings live in `settings.toml` (in `~/.config/kadr` on Linux), with a comment on every setting, so they can be changed by hand or kept with your dotfiles. kadr picks up edits the next time it needs them, and the settings window keeps your comments and changes only what you change there. Settings → About → Settings file opens it. If the file can't be read, kadr says where in the settings window and leaves it as it is. The old `settings.json` is moved over by itself.

## [0.3.0-alpha.2] - 2026-10-26

Recording and window shots come to macOS and Windows, and Windows gets an installer. Both systems still pass only CI's tests; nobody has run them on a real machine yet.

### Added

- Screen recording on macOS and Windows, through the system's own capture and encoder, with the system's sound and the default microphone, at a steady frame rate.
- Window shots on every system: a clicked window is taken by itself, whatever covers it. With the new Window background setting it goes straight onto a background saved in the editor, which the editor can still change.
- Every screen in one shot (`kadr shot all`), the pointer in screenshots, and a self-timer of 3, 5 or 10 seconds.
- Windows: an installer for your user only (the Start menu, the tray at login, `.kadr` files, an uninstaller) and a portable `kadr.exe` in one file, instead of a zip. No console window opens with kadr.
- A crash leaves a note in `crash.log` in kadr's cache folder, to send with a bug report.

### Changed

- The Linux tray icon takes the bar's colour, so it shows on a light or dark bar.

### Fixed

- The studio played the end of a recording's sound only if it kept up with the picture: playback stopped the sound along with the last frame. It now plays out, and runs closer to the picture.
- Recordings no longer lose their sound at the edges: the first half second, which was muted to hide the microphone's click, is now 0.15 s, and after Stop the recorder runs on for 0.4 s so the last word, still in the system's audio buffers, makes it into the file.
- kadr on Windows starts about 2 seconds sooner.
- On macOS kadr stopped answering after its first system notification.
- On Windows the cards had a tall outline and shadow round them.
- A screen or window capture the system doesn't answer ends in an error card instead of a frozen kadr.

## [0.3.0-alpha.1] - 2026-10-23

The first public build. Linux (Hyprland) is the one kadr is used on every day; the macOS and Windows builds are new and haven't been tried on a real machine yet.

### Added

- Screenshots of an area, a window or the whole screen, and long scrolling ones. After each capture a thumbnail shows in the corner to copy, edit, pin on top or drag into a chat.
- The screenshot editor: arrows, boxes, text, step numbers, a highlighter, pixelation, a spotlight and cropping, on a background with padding, rounded corners and a shadow. A `.kadr` project file keeps an edit to open again later.
- Screen recording on Linux, with the microphone, system audio, the camera, clicks and keys. The studio turns a recording into a finished video: zooms on clicks, cuts, a smooth large cursor, MP4 or GIF.
- Text recognition, a history of captures, English and Russian, dark and light themes that follow the system.
- macOS and Windows: the editor, the studio, screenshots, the area picker, the tray icon and global hotkeys. Recording and picking windows aren't there yet.
- A check for new releases: once a day the tray looks, and a card links to the new release's page. Nothing is downloaded; it can be turned off in Settings.

[Unreleased]: https://github.com/evlaso/kadr/compare/v0.3.0-alpha.2...HEAD
[0.3.0-alpha.2]: https://github.com/evlaso/kadr/compare/v0.3.0-alpha.1...v0.3.0-alpha.2
[0.3.0-alpha.1]: https://github.com/evlaso/kadr/releases/tag/v0.3.0-alpha.1
