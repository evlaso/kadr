# Studio

The studio turns a screen recording into a finished video: it zooms in where you clicked, cuts out the dull parts, draws a large smooth cursor, puts the video on a background with the camera in a corner, and exports MP4 or GIF.

It works best with a recording made in studio mode (the sparkle button on the recording bar): then the system's cursor stays out of the video, kadr draws its own, and the camera is recorded separately, so all of them can be changed here. See [Recording](record.md).

kadr keeps your edit as you go: open the same recording again and it's all there. **Edited** next to the name in the title bar says it's changed. **Project** saves the recording with its edit as a `.kadr` file, next to the recordings (see [Project files](projects.md)).

## How to open

- **From the card** after a recording: **Studio**.
- **After a studio mode recording** the studio opens by itself.
- **After every recording:** turn on Settings → Recording → **Open the studio**.
- **From History:** select a recording and press E, or click **Studio**.
- **From the command line:** `kadr studio <video|project.kadr>`. With `--render` kadr exports right away with the saved edit, without opening the window.
- **From the file manager:** a `.kadr` project of a recording opens in the studio.

## The window

- **The rail** at the left has the sections: Zoom, Cursor, Keyboard shortcuts, Audio, Camera, Background, and Cut apart at the bottom. A click opens a section's settings beside the rail.
- **The stage** in the middle plays the video as it will look, with every effect.
- **The timeline** at the bottom: the ruler, the filmstrip, the zoom track under it and the cut row at the very bottom. A click on the ruler or the filmstrip moves the playhead. Above it: **To the start**, play and pause, **To the end**, and two **Timeline zoom** buttons.
- **The title bar:** the file's name, **Project** and **Export**.

## Zooms and cuts

**Zoom.** A studio mode recording comes with zooms already placed on your clicks; for any other recording, add them yourself.

- Click the zoom track to add a zoom. Drag a zoom to move it, drag its edges to make it longer or shorter.
- **Zoom level** sets how close the selected zoom gets (100% to 400%); **Apply zoom level to all** gives every zoom the same level.
- **Zoom mode** of the selected zoom: **Follow cursor**, or **Manual**: then click the video where to zoom.
- **Delete zoom** (or Delete) removes the selected one.
- **Automatic → Zooms on clicks** places zooms on the clicks again, in place of the ones there. It needs a recording with clicks.
- **Motion blur:** **Off**, **Light** or **Strong**, on zooms, camera moves and a fast cursor. It shows in the preview and the export.

**Cut.**

- The filmstrip's yellow edges trim the start and the end.
- Drag across the cut row to cut that piece out. Drag a cut to move it, drag its edges to change it. X cuts a second at the playhead.
- With a cut selected, the Cut section says what it removes; **Restore the piece** (or Delete) brings it back. **Restore the whole video** drops the trim and every cut.

## Cursor, shortcuts, camera and audio

**Cursor:**

- For a studio mode recording: **Show the cursor**, its **Size** (0.5× to 3×), **Style** (**Arrow**, **Light arrow** or **Dot**) and **Motion** (**Natural** or **Smooth**). Any other recording keeps the cursor as it was recorded.
- **Clicks:** **Ripple animation** (rings where you clicked), and with kadr's own cursor, **Press effect** (the cursor reacts to a click).

**Keyboard shortcuts:** **Show shortcuts** puts a badge with the shortcuts you pressed on the video, at the **Bottom** or the **Top**, in size S, M or L. It needs a recording made with **Keyboard shortcuts** on in the recording bar.

**Camera** (when the recording has one): **Show the camera**, its **Size**, **Rounding** from a square to a circle, and where it sits: one of eight spots round the edge.

**Audio** (when the recording has sound): **Audio** on or off, **Volume** (up to 200%), **Channels**: **Stereo** or **Mono**. GIFs have no audio.

## Background and export

**Background:**

- **Presets** at the top: + saves the current look (background, padding, cursor, export), the dropdown picks, renames or deletes one.
- **Gradients**, **Wallpapers** (the Omarchy themes' backgrounds, where there are any, plus **Your own image…**) and **Plain color**.
- **Aspect ratio:** **Auto** (the recording's own), 1:1, 5:4, 4:3, 3:2, 16:9, and the tall 4:5, 3:4, 9:16.
- **Padding**, **Corners** and **Shadow**.

**Export.** The **Export** button opens the export settings:

- **Format:** MP4 or GIF.
- **Size:** 720p, 1080p, 1440p or 4K.
- **FPS:** **Source** (as recorded), 30, 60 or 120.
- **Quality** (MP4 only): **Best**, **High**, **Medium** or **Small**.
- Under them, the video's size and length and how big the file will be.
- **Preview** builds a quick 720p video with every effect and plays it in mpv. **Export** (or Ctrl+S) closes the studio and makes the video, saved next to the recording with `-studio` added to its name. Its progress shows on a card over a blurred frame, with a cancel button, and the card becomes the result.

A new recording starts with the look you used last in the studio.

## Platforms

Clicks and keys are logged by kadr's Hyprland hook, so on Linux only for now. On macOS and Windows a recording has no click or key log yet: zooms go in by hand, and click rings and shortcut badges have nothing to show.

## Keys

Ctrl is Cmd on macOS.

| Keys | Does |
|---|---|
| Space | Play or pause |
| ← and → | Back or forward half a second |
| Home, End | To the start or the end |
| X | Cut a second at the playhead |
| Delete, Backspace | Delete the selected zoom, or restore the selected cut |
| Ctrl+S | Export |
| Esc | Close the export settings or the aspect ratio menu, stop placing a zoom, let go of the selection; with nothing to let go of, close the studio |

## Settings

The Studio section of the settings sets where a new edit starts:

- **Default zoom** for zooms placed on clicks: ×1.25, ×1.5, ×1.8, ×2.2 or ×3. In `settings.toml`: `studio_zoom`.
- **Default background:** one of the studio's gradients or colours. In `settings.toml`: `studio_backdrop`.
- **Default padding:** None, S, M or L. In `settings.toml`: `studio_padding`.

Once you've edited something in the studio, the look you used last wins over the default background and padding.

- **Open the studio** (Settings → Recording → After a recording): open every new recording in the studio. Studio mode recordings always open in it. In `settings.toml`: `after_video.edit`.

## Commands

```
kadr studio <video|project.kadr> [--render]    studio: zooms, background, clicks and keys (--render exports at once)
```
