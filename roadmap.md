# Roadmap

The goal: as good and as complete as the best dedicated screenshot and screen-recording tools (capture,
the picker, the editor, recording, the studio), and clearly better than what the systems ship with.
Plus what nobody has: Linux on Wayland as a first-class platform, native speed with no webview, agent
mode (MCP), and open source.

Status marks: ✅ there · 🟡 partly · ⬜ not yet · ❌ won't do · — doesn't apply to the platform.
Platforms: L is Linux (Wayland, Hyprland), M is macOS, W is Windows. ✅ on M or W means shared code with
no part of its own for that system, or something desktop-e2e has checked; where there is code for that
system that hasn't run there yet, it's 🟡. The platform columns are kept up to date from the code.

Architecture, budgets and tests are in the spec, [SPEC.md](SPEC.md); this file says only what
comes and in what order.

## Milestones

| Version | Theme | Sections | State | Done when |
|---|---|---|---|---|
| 0.3 | Public alpha |  | 🟡 | All three systems build and pass desktop-e2e, the update check works, the first testers run it on real macOS and Windows machines |
| 0.4 | Screenshot parity | 1–5 | ⬜ | Every row of sections 1–5 is ✅ on all three systems (L, M and W), including all that is on L only today; full self-update (§12.3) |
| 0.5 | Recording and studio parity | 6–7 | ⬜ | Every row of sections 6–7 is ✅; export is not slower than the tools it's measured against, on the same hardware |
| 0.6 | Agent mode | 8 | ⬜ | Section 8: MCP, markers, script, voice-over, subtitles |
| 0.7 | Beyond parity | 9 | ⬜ | Section 9: scrolling everywhere, sharing to your own storage, URL scheme and CLI API, localisations |
| 1.0 | Stability |  | ⬜ | The spec's §12.1: real users on all three systems, frozen formats, two self-updating releases in a row |

Order inside a milestone: first what people see every day (the picker, the card, the editor), then
the rare things. A feature counts as done only with dev hooks and a test (§6).

## 1. Capture

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| An area, a window, the whole screen, every screen | ✅ | ✅ | ✅ | `kadr shot all`; windows on M/W from the system's list (W: EnumWindows + WindowFromPoint) |
| All in one: one shortcut, switch the mode on the fly, remember the last area | ✅ | ✅ | ✅ | M/W: the capture bar, the arrows and a shot from it pass desktop-e2e |
| Exact area size: type W×H, a fixed ratio, nudge with the arrow keys | ✅ | ✅ | ✅ |  |
| Crosshair, a magnifier with pixels and coordinates | ✅ | ✅ | ✅ |  |
| The screen frozen while you select | ✅ | ✅ | ✅ |  |
| Self-timer (3/5/10 s) | ✅ | ✅ | ✅ | picked in the settings; takes the whole screen |
| A window on a background: padding, wallpaper, colour or picture, shadow; a window on a transparent background | ✅ | ✅ | ✅ | the background is a preset saved in the editor (the Window background setting), and the editor opens the shot with it; M/W: pass desktop-e2e |
| A window shot without the windows covering it (the window's own buffer) | ✅ | ✅ | ✅ | L: ext-image-copy-capture; M: CGWindowList; W: its own WGC frame; e2e on all three |
| Scrolling capture: down, sideways, any app | ✅ | ✅ | ✅ | the person scrolls, kadr stitches; sideways checked by the stitching test; M/W: the frame and the bar are separate windows, pass desktop-e2e |
| The pointer in the screenshot (an option) | 🟡 | ✅ | ✅ | L: through `grim -c`; the nested session has no pointer, so it's unchecked |
| A shot from the command line or a shortcut with no UI, straight to the clipboard or a file | ✅ | ✅ | ✅ |  |

## 2. The card

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| A card after a capture: open, edit, copy, save, delete | ✅ | ✅ | ✅ |  |
| Drag from the card into any app (as a file) | ✅ | 🟡 | 🟡 | M/W: unchecked |
| Quick actions you choose: copy, edit, pin, text recognition, share | 🟡 | 🟡 | 🟡 | can't be chosen; no sharing |
| Several cards in a stack, bring back a closed one | 🟡 | 🟡 | 🟡 | no bringing back |
| Position (corner, monitor), size, closing on a timer, gestures | 🟡 | 🟡 | 🟡 | no monitor or size |
| Hide all cards for a while | ⬜ | ⬜ | ⬜ |  |
| File info (size, resolution) | ⬜ | ⬜ | ⬜ |  |

## 3. Editor

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| Arrow: 4 styles, curved too | ✅ | 🟡 | 🟡 | filled head, open head, both ends, tapered, all of them bend; M/W: drawing checked by golden tests in CI, picking a style only by e2e on Linux |
| Line, box, filled box, ellipse | ✅ | ✅ | ✅ |  |
| Pen with smoothing, highlighter that finds the text line's height | ✅ | 🟡 | 🟡 | the highlighter picks its width and keeps to the line from the screenshot's pixels, no OCR; leaving the line lets go, Shift turns it off; M/W: shared code, UI unchecked |
| Text: 7 ready styles, a plate, an outline | ✅ | 🟡 | 🟡 | plain, plate, outline, shadow, marker, pill, label; M/W: drawing checked by golden tests in CI, picking one only by e2e on Linux |
| Step numbers | ✅ | ✅ | ✅ |  |
| Spotlight (dim everything but an area) | ✅ | ✅ | ✅ |  |
| Blur (secure and smooth), pixelation (randomised) | ✅ | ✅ | ✅ | one blur, and a safe one: blocks with noise first, then the blur; the rasteriser is shared, golden tests run in CI on all three |
| Black box (can't be undone in the file) | ✅ | 🟡 | 🟡 | M/W: Hide sensitive needs tesseract; desktop-e2e hasn't drawn a box yet |
| Crop: ratios, snapping to edges | ✅ | ✅ | ✅ |  |
| Turn, flip, resize | ✅ | ✅ | ✅ | resizing is the Image size menu |
| Colour: pipette, your own colours, recent colours | ✅ | 🟡 | 🟡 | M/W: shared code; desktop-e2e hasn't used the pipette yet |
| Join several pictures by drag and drop | ✅ | 🟡 | 🟡 | dropped beside the screenshot, a picture joins it on that side (as tall or as wide); dropped on it, goes on top; M/W: dropping files unchecked |
| Background and frame: 20 backgrounds, your own, padding, alignment, ratio, Even margins, presets | ✅ | 🟡 | 🟡 | position is a 3×3 grid, at an edge the picture sits flush and square there; Even margins cuts plain margins to match; M/W: drawing checked by golden tests in CI |
| Project file (`.kadr`), editing again later | ✅ | ✅ | ✅ |  |
| Unlimited undo and redo, step by step | ✅ | ✅ | ✅ |  |
| Export: PNG/JPEG/WebP, 1×/2× scale, copy, save, share, a handle to drag the result out | 🟡 | 🟡 | 🟡 | no sharing; PNG, JPEG and WebP by the name in Save As; original / 1× / 2× at the editor's bottom, with HiDPI in mind, for saving, copying and dragging; the choice is kept in the project; M/W: scale UI unchecked |
| Dark and light theme, following the system | ✅ | ✅ | ✅ |  |

## 4. Pin on top, history, text recognition

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| Pin on top: size, opacity, move by a pixel with the arrows, let clicks through | ✅ | 🟡 | 🟡 | M/W: unchecked; letting clicks through is in doubt |
| History: recent screenshots and recordings, filter by kind, delete, keep for N days | ✅ | ✅ | ✅ |  |
| Text recognition: select an area, the text goes to the clipboard; 30+ languages; on the device | 🟡 | 🟡 | 🟡 | tesseract, installed separately on M/W |
| Native text recognition: Vision on macOS, Windows.Media.Ocr on Windows, tesseract on Linux | — | ⬜ | ⬜ |  |
| QR codes | ✅ | 🟡 | 🟡 | zbarimg, installed separately on M/W |
| Text actions: copy an e-mail or a phone number, hide personal data | 🟡 | 🟡 | 🟡 | only Hide sensitive and a link from text recognition |

## 5. Platform and behaviour

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| Tray or menu bar icon with quick actions, start at login | ✅ | ✅ | ✅ |  |
| Global shortcuts that catch conflicts, hints in the system's style | 🟡 | 🟡 | 🟡 | conflicts aren't caught |
| Platform edges (§3.10): dialogs, menus, keyboard, input methods | ✅ | ✅ | ✅ | on L kadr draws the menus itself, as the spec says |
| Several monitors with different DPI | 🟡 | 🟡 | 🟡 |  |
| Settings: where to save, file name template, format, clipboard, sound, what to do after a capture | 🟡 | 🟡 | 🟡 | no format |
| Self-update with a What's new screen (§12) | 🟡 | 🟡 | 🟡 | Stable and Beta channels, download, signature, rollback; no What's new screen yet: the update card links the release page; not yet run on M or W |
| macOS permissions: a clear first-run request for screen recording and accessibility | — | ⬜ | — |  |
| Packages: dmg (signed), msi/winget, AUR, Flatpak, Homebrew | 🟡 | 🟡 | 🟡 | L: tar.xz; M: unsigned dmg; W: an installer (Inno Setup, no admin) and a portable .exe, unsigned; no winget |

## 6. Recording

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| Record an area, a window or the screen; MP4 (H.264) and GIF | 🟡 | 🟡 | 🟡 | a window is recorded as an area; M/W: the screen is checked in CI, an area isn't |
| Recording on macOS (ScreenCaptureKit) and Windows (WGC), not stubs | — | ✅ | ✅ | both record in desktop-e2e, at a steady frame rate |
| Quality, FPS, resolution; hardware encoding | 🟡 | 🟡 | 🟡 | no resolution |
| Microphone and system sound, picking devices, a level meter | ✅ | 🟡 | 🟡 | M/W: the default device only; M: the microphone from macOS 15; the meter unchecked |
| Camera over the recording: position, size, shape, full screen | 🟡 | ⬜ | ⬜ | M/W: the camera goes through v4l2, so none there |
| Clicks and keys in the recording (colour, size, animation; modifiers only) | 🟡 | ⬜ | ⬜ | L: logged by the Hyprland hook, drawn in the studio |
| Show or hide the pointer | 🟡 | 🟡 | 🟡 |  |
| Do Not Disturb on its own, hide the desktop, recording time in the tray | ⬜ | ⬜ | ⬜ |  |
| Pause and resume, cancel and delete | ✅ | 🟡 | 🟡 | M/W: in the code, unchecked |
| Start timer, countdown | ✅ | 🟡 | 🟡 | M/W: unchecked |

## 7. Studio

| Feature | L | M | W | What's missing |
|---|---|---|---|---|
| Timeline: trim, cuts, dragging pieces around | ✅ | 🟡 | 🟡 | M/W: the studio hasn't been tried on a real machine; export needs ffmpeg with libplacebo |
| Zooms by hand; **automatic zooms that follow the pointer, as suggestions** | 🟡 | 🟡 | 🟡 | by hand and from the recording's clicks (no clicks on M/W) |
| Panning between zooms, smooth easing | 🟡 | 🟡 | 🟡 |  |
| Speed: faster or slower pieces, cutting out pauses | ⬜ | ⬜ | ⬜ |  |
| Pointer: smoothing, motion blur, size, style, click effect | ✅ | 🟡 | 🟡 | M/W: no pointer log from the recording |
| Background and frame: wallpapers, gradients, your own, corners, shadow, padding, presets | ✅ | 🟡 | 🟡 | M/W: unchecked |
| Export in ratios: 16:9, 1:1, 9:16, with zooms that follow the crop | 🟡 | 🟡 | 🟡 | zooms don't follow the crop |
| Subtitles: an editable track, burned in, SRT/VTT beside the video | ⬜ | ⬜ | ⬜ | see 8 |
| Annotations on the video (arrow, text, highlight, timed) | ⬜ | ⬜ | ⬜ |  |
| The camera after recording: position, shape, on and off by piece | 🟡 | ⬜ | ⬜ | not by piece; M/W: no camera recording |
| Export: MP4/GIF/WebM, quality, a preview of the result, progress as a card | 🟡 | 🟡 | 🟡 | no WebM |
| A `.kadr` project with the video, back to the edit later | ✅ | ✅ | ✅ |  |

## 8. Agent mode, kadr's own (SPEC §11)

| Feature | Status |
|---|---|
| `kadr mcp`: capture / windows / record.start / record.marker / record.stop / studio.export / ocr | ⬜ |
| Markers in a recording become zooms, chapters, timing | ⬜ |
| `script.json` in the project: steps, voice-over text | ⬜ |
| Voice-over by a local TTS from the script; a cloud one as an option | ⬜ |
| Subtitles from the script (no speech recognition) | ⬜ |
| Confirming in the UI that an agent may see the screen | ⬜ |
| Script templates: "screencast of changes", "audit", "test", "onboarding", "tutorial" | ⬜ |

## 9. Beyond parity

| Feature | Status | Why |
|---|---|---|
| Scrolling capture on macOS and Windows | ✅ | The systems' own tools lack it; now on all three, M/W pass desktop-e2e (see section 1) |
| Sharing to your own storage: S3-compatible, WebDAV, your own script; the link to the clipboard | ⬜ | Instead of a vendor's cloud: no service of our own, no accounts |
| URL scheme (`kadr://capture?area=…`) and a stable CLI API | ⬜ | Automation from launchers and scripts |
| Localisations beyond en/ru by the community (Weblate or PRs) | ⬜ | Open source |
| Accessibility: a screen reader in the settings and the editor | ⬜ | The debt of a UI of our own, §3.10 |
| Flatpak and Snap, AppImage | ⬜ | Linux beyond Arch |

## Not planned

- A cloud of our own, accounts, teams, SSO: that's a hosting business, not kadr. Sharing goes only to the user's own storage.
- Speech transcription in videos: subtitles come from the script (§11); recognition not before 1.0, and only local.
- Webview, Electron, a built-in browser: never.

## Next steps

1. Testers on macOS and Windows, fixes from their reports, then `0.3.0`.
2. Section 6 on macOS and Windows: picking the microphone, the camera and the click log.
3. For `0.4`, everything that is on L only comes to all three systems, in this order:
   1. **Editor**: the rows of section 3 to ✅.
   2. **Card**: the rows of section 2 to ✅.
   3. **Capture**: the pointer in screenshots checked on L.
   Each row is its own commit with a test.
4. Full self-update (§12.3) for `0.4.0`: done; it still needs a run on real macOS and Windows machines.
