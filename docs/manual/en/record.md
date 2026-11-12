# Recording

Records an area or the whole screen to an MP4, with the microphone, the computer's sound, your camera and the shortcuts you press. In studio mode the recording opens straight in the [Studio](studio.md), which turns it into a finished video. Smaller jobs, trimming and making a GIF, are done right from the card after a recording.

## How to open

- **The hotkey.** On macOS and Windows the default is Ctrl+Shift+2; on Linux kadr starts from your Hyprland bindings. Record the previous area can get a hotkey of its own in Settings → Shortcuts.
- **The capture bar** (`kadr capture`): Recording, or 8. See [Capture](capture.md).
- **The tray menu:** Record the screen.
- **The command** `kadr record`. Pressing the hotkey (or running `kadr record`) again closes the recording bar, or stops a recording that's running.

First you pick the area, the same way as for a screenshot (see [Capture](capture.md)): drag a box, or click a window to record it, or click elsewhere to record the whole screen. The recording bar then shows by the area as soon as you let go of the mouse.

## The recording bar

The bar sits under the area (or above it, or inside it, where there's no room), and the rest of the screen is dimmed. From the left:

- **Cancel** (Esc).
- **Area size:** the area's size in pixels. Click it for a menu of sizes: 640 × 480, 1280 × 720, 1920 × 1080, 1080 × 1080, 1080 × 1920, or Fullscreen.
- **Microphone:** a click turns it on or off. While it's on, the button fills from the bottom with how loud you are, so you can see you're heard. The arrow beside it chooses the microphone (the system's default, or another one).
- **System audio:** what the computer plays.
- **Camera:** shows the camera bubble over the area (see below).
- **Keyboard shortcuts:** logs the shortcuts you press, for the studio to show as badges.
- **Studio mode:** on by default, see below.
- **Countdown before recording:** off, 3 or 5 seconds, counted over the area. Esc during the count cancels.
- **Another area** (R): pick the area again.
- **Fullscreen** (F): record the whole screen instead.
- **Record** (Enter).

The area stays adjustable while the bar is up: drag the knobs on its corners and edges, drag it from inside, move it with the arrows, or resize it with Alt+arrows. The bar remembers its choices for the next recording.

**Studio mode** records for the [Studio](studio.md): the system cursor stays out of the video, and kadr logs the pointer, so the studio draws its own cursor, large and smooth, and places zooms on your clicks. The camera is recorded separately. When you stop, the recording opens in the studio instead of showing a card.

## While recording

A small bar stays by the area, outside it, so it isn't recorded. It shows a red dot and the time, the microphone's level, and:

- **Pause** and **Resume**: the paused time is left out of the file.
- **Stop and save**.
- **Start over**: throws the take away and records the same area again.
- **Stop and delete**.

A red frame marks the recorded area and the rest of the screen is dimmed; neither is recorded, and clicks go through to the windows below. Frame around the area in the settings turns this off.

When the whole screen is recorded, the bar goes to another monitor. With only one monitor there's nowhere to put it: stop with the hotkey, or `kadr record stop`.

## The camera bubble

Turn on Camera in the recording bar, and a live picture of your camera shows over the area. Set it up before recording:

- Drag it: it snaps to a corner or to the middle of an edge.
- Hover over it for Smaller, Bigger and Shape. Shape steps through a circle, rounded corners and a square.
- Scroll over it, or drag the band round it, to resize it.

kadr remembers the look for the next recording, and the studio starts from it. Without studio mode the bubble is recorded where you put it. In studio mode the camera goes to a file of its own, and the studio shapes, sizes and places it.

## After recording

Without studio mode a card shows in the corner of the screen, with the recording's length. The file is copied to the clipboard by default, to paste into a chat or a folder. Hover over the card for:

- **Copy**: the file, to the clipboard.
- **Studio**: opens the [Studio](studio.md). Its tooltip says what you'll get: "Zooms follow your clicks" when kadr logged them, "Background and zooms by hand" when it didn't.
- **Trim** (top right): the trim window, below.
- **GIF** (bottom right): Save as GIF, the whole recording, 720 pixels wide at 15 fps.
- **Show in folder**, **Delete file** (to the trash) and **Close**.

A double-click opens the recording in your video player. Drag the card into a chat or a file manager to drop the file there. While a GIF or a trim is being made, a card shows its progress, with Cancel, and then turns into the result's card.

### The trim window

The trim window (`kadr trim`) shows a frame of the video over a filmstrip. Drag the start and end handles on the filmstrip; a click moves the nearer one there. The frame follows, and "Selected" shows the length.

- **Play** (Space) plays the selected piece in mpv.
- **Save** (Ctrl+S, Cmd on macOS) cuts it into a new file with "-trim" in its name; it works once something is trimmed. The original stays.
- **Save GIF** (G) makes a GIF of the selected piece: 480, 720 or 960 pixels wide, at 10, 15 or 24 fps.
- **Cancel** (Esc).

`kadr trim <video> --from S --to S` trims without the window, and `kadr gif <video>` makes a GIF of the whole recording, 720 pixels wide at 15 fps.

## On macOS and Windows

Recording there goes through the system's own capture and encoder, with the system's sound and the default microphone. Choosing another microphone, its level meter and the camera are Linux-only for now. So is the click and key log, which comes from the Hyprland hook: the studio places zooms on clicks only on Linux. Nobody has recorded on a real Mac or Windows machine yet.

## Keys

| Keys | Does |
|---|---|
| Enter | Record (in the recording bar) |
| Esc | Cancel the recording bar (or close its size menu first), cancel the countdown, or close the trim window |
| R | Another area |
| F | The whole screen |
| Arrows | Move the area a pixel |
| Shift+arrows | Move it ten pixels |
| Alt+arrows | Resize it from the right and bottom edges |
| Space | Play the selection (trim window) |
| G | Save a GIF (trim window) |
| Ctrl+S (Cmd+S on macOS) | Save the trimmed piece (trim window) |

## Settings

| Setting | Key | Does |
|---|---|---|
| Recording → Show the thumbnail | `after_video.card` | The card after a recording |
| Recording → Copy to the clipboard | `after_video.copy` | Copy the file right away |
| Recording → Save to the folder | `after_video.save` | Off, the file goes to a temporary folder and is gone after a restart |
| Recording → Open the studio | `after_video.edit` | Open every recording in the studio; studio mode recordings always open in it |
| Recording → Recordings folder | `video_dir` | Where recordings go |
| Recording → Frame rate | `fps` | Source (the screen's refresh rate), 30, 60 or 120 fps |
| Recording → Quality | `quality` | Medium, High, Very high or Maximum; higher keeps small text sharp but makes bigger files |
| Recording → Frame around the area | `region_frame` | Outline the recorded area (the outline isn't recorded) |

How long the card stays and where it shows are in Settings → General (see [Capture](capture.md)).

## Commands

```
kadr record                                    the recording bar (or stop a running recording)
kadr record region|fullscreen [--mic] [--audio] [--webcam] [--keys] [--studio]
                                               start recording at once (again to stop)
kadr record area "X,Y WxH"                   the recording bar for a given area
kadr record previous                           the recording bar for the last area picked
kadr record stop                               stop recording
kadr trim <video> [--from S --to S]            trim a recording (no flags: the trim window)
kadr gif <video>                               save a recording as GIF (720px, 15 fps)
```
