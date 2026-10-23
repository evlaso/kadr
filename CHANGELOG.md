# Changelog

What changed in each kadr release, for the people who use it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/). In Russian: [docs/CHANGELOG.ru.md](docs/CHANGELOG.ru.md).

## [Unreleased]

## [0.3.0-alpha.1] - 2026-10-23

The first public build. Linux (Hyprland) is the one kadr is used on every day; the macOS and Windows builds are new and haven't been tried on a real machine yet.

### Added

- Screenshots of an area, a window or the whole screen, and long scrolling ones. After each capture a thumbnail shows in the corner to copy, edit, pin on top or drag into a chat.
- The screenshot editor: arrows, boxes, text, step numbers, a highlighter, pixelation, a spotlight and cropping, on a background with padding, rounded corners and a shadow. A `.kadr` project file keeps an edit to open again later.
- Screen recording on Linux, with the microphone, system audio, the camera, clicks and keys. The studio turns a recording into a finished video: zooms on clicks, cuts, a smooth large cursor, MP4 or GIF.
- Text recognition, a history of captures, English and Russian, dark and light themes that follow the system.
- macOS and Windows: the editor, the studio, screenshots, the area picker, the tray icon and global hotkeys. Recording and picking windows aren't there yet.
- A check for new releases: once a day the tray looks, and a card links to the new release's page. Nothing is downloaded; it can be turned off in Settings.

[Unreleased]: https://github.com/vlasmn/kadr/compare/v0.3.0-alpha.1...HEAD
[0.3.0-alpha.1]: https://github.com/vlasmn/kadr/releases/tag/v0.3.0-alpha.1
