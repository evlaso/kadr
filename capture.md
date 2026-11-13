# Capture

Everything that takes a picture of the screen: an area, a window, the whole screen or every screen, a long scrolling shot, a shot on a self-timer. After a capture a card shows in the corner of the screen, where you copy the picture, open it in the [Editor](editor.md), pin it on top of your windows or read the text in it. Text recognition and pinning live here too.

## How to open

- **The capture bar** (`kadr capture`) puts every mode on one bar, so one hotkey is enough. It has no default hotkey: set one in Settings → Shortcuts → All modes on one bar.
- **Hotkeys.** On macOS and Windows the defaults are Ctrl+Shift+1 for an area, Ctrl+Shift+3 to recognize text and Ctrl+Shift+6 for a scrolling screenshot. On Linux kadr starts from your Hyprland bindings. More actions (the previous area, a window or area, the screen) can get a hotkey in Settings → Shortcuts.
- **The tray menu:** Screenshot, Recognize text, Scrolling screenshot.
- **Commands:** `kadr shot`, `kadr shot scroll`, `kadr text`, `kadr pin`; see [Commands](#commands).

## The capture bar

The bar shows near the bottom of the screen with eight modes in three groups:

| Key | Mode | Does |
|---|---|---|
| 1 | Area | Pick an area |
| 2 | Previous | Take the last area again, as it is now |
| 3 | Fullscreen | The whole screen the focus is on |
| 4 | Window | Point at a window and click it |
| 5 | Scrolling | A long scrolling shot |
| 6 | Timer | The whole screen, after a countdown |
| 7 | Text | Pick an area and recognize the text in it |
| 8 | Recording | The recording bar (see [Recording](record.md)) |

Click a mode, press its digit, or move with the left and right arrows and press Enter. Esc, a click outside the bar or the same hotkey again closes it.

## Picking an area

![The area picker keeps an 800 × 450 selection, its size label, adjustment handles and key hints over a drawn sample](https://evlaso.github.io/kadr/images/capture-light.webp)

The screen freezes, and you drag a box over it. A hint bar says what the keys do at each moment.

- **Drag** to draw the area; it's taken when you let go. Shift while dragging makes a square. Holding Space while dragging moves the whole box instead.
- **Click** without dragging: over a window it takes that window; over nothing it copies the colour under the pointer as a hex code (#RRGGBB) and kadr says "Color copied".
- **Space** switches to windows only: a click takes the window under the pointer, and nothing is drawn. Space again goes back to drawing an area.
- **Keep the area to adjust it.** Press an arrow, A or P, or a digit, while the box is there, and it stays after you let go of the mouse. Then drag its handles to resize it, drag it from inside to move it, or double-click inside it to take it.
- **Arrows** move the area a pixel, Shift+arrows ten. **Alt+arrows** grow or shrink it from the right and bottom edges.
- **A** steps through aspect ratios: Free, 1:1, 4:3, 16:9, 9:16.
- **Digits** type an exact size in screen pixels: the width, then X (or Tab or Space) and the height, then Enter. Left without a height, it follows the aspect ratio.
- **P** brings back the last area picked on this screen, to adjust. Enter with nothing drawn takes that area straight away.
- **Enter** takes the area; **Esc** cancels.

The Area mode and `kadr shot region` don't snap to windows: a click there only copies a colour.

## Windows, screens and scrolling

- **A window** comes by itself, whatever covers it, with rounded corners and a soft shadow on a transparent background. Window shadow in the settings turns the shadow off. Window background puts it straight onto a background saved in the editor instead; the editor opens it with that background, still to change. Window shots work on Linux, macOS and Windows.
- **Fullscreen** takes the screen the focus is on.
- **Every screen at once** (`kadr shot all`): all monitors in one picture, laid out as your desktop has them.
- **The self-timer** counts 3, 5 or 10 seconds over the screen (Self-timer in the settings), then takes it. Esc cancels the countdown.
- **The pointer** can be in the picture: turn on Pointer in screenshots.

**A scrolling shot:** pick an area, then scroll it yourself. kadr keeps grabbing the area and stitches what comes into view into one long picture. It works down a page and sideways, to the right (a timeline, a wide table), which makes one wide picture.

A frame shows round the area, and a bar by it says "Scroll — kadr stitches it" with the length so far in pixels. Press Done, or the same hotkey again, to save it; Cancel throws it away. Scrolling shots work on Linux, macOS and Windows.

## The card after a capture

![A sample screenshot's quick-access card with Copy, Edit, pin, text recognition and file actions visible](https://evlaso.github.io/kadr/images/card-light.webp)

A card with the picture slides in at the corner of the screen. By default the screenshot is also copied to the clipboard right away. Hover over the card for its buttons:

- **Copy**: puts the picture on the clipboard and closes the card.
- **Edit**: opens it in the [Editor](editor.md). A double-click on the card does the same.
- **Pin on top** (top right): keeps it above every window, see below.
- **Recognize text** (bottom right): reads the text in the picture.
- **Show in folder** (bottom right).
- **Delete file** (bottom left): moves the file to the trash.
- **Close** (top left).

Drag the card into a chat, a browser or a file manager to drop the file there. Swipe it toward the screen edge to put it away.

A card goes after 10 seconds by default, and stays while the pointer is over it. How long, which corner and how many cards at once are in Settings → General.

## Pinning

A pinned picture floats above every window, at its real size, so you can keep a reference in sight while you work. Pin a screenshot from its card, from the editor's bottom bar, from [History](history.md) (P), with `kadr pin`, or have every screenshot pinned (Pin on top under After a screenshot).

- Drag it to move it; arrows nudge it a pixel, Shift+arrows ten.
- Scroll to zoom it; Alt+scroll or Ctrl+scroll changes its opacity.
- **L** lets clicks through to the windows below it. Only the lock in its corner still takes clicks; press it, or L, to catch clicks again.
- Esc, a double-click or a right-click closes it.

## Text recognition

`kadr text` freezes the screen; pick an area, and its text goes to the clipboard. A card shows what was found, with Copy again and, when the text holds a link, Open link. QR codes are read too, when zbar is installed. With a picture (`kadr text <image>`, or Recognize text on a card) kadr reads the whole picture.

kadr reads English and Russian, and Ukrainian, German, French and Spanish too when Tesseract has them. Russian needs Tesseract's Russian data: on Arch it's the `tesseract-data-rus` package, or Settings → Text → Russian downloads it (15 MB) into kadr's own folder, without sudo.

## Keys

### The capture bar

| Keys | Does |
|---|---|
| 1–8 | Run that mode |
| Left, Right | Move between modes (Tab moves right too) |
| Enter or Space | Run the chosen mode |
| Esc | Close the bar |

### Picking an area

| Keys | Does |
|---|---|
| Enter | Take the area (or the previous one, if it's shown) |
| Esc | Cancel |
| Space | Windows only, or back to drawing an area |
| Space, held while dragging | Move the box being drawn |
| Shift, while dragging | A square |
| A | Next aspect ratio |
| P | The previous area |
| Arrows | Move the area a pixel |
| Shift+arrows | Move it ten pixels |
| Alt+arrows | Resize it from the right and bottom edges |
| Digits, then X and digits, then Enter | An exact size |

### A pinned picture

| Keys | Does |
|---|---|
| Esc | Close it |
| L | Let clicks through, or catch them again |
| Arrows | Move it a pixel |
| Shift+arrows | Move it ten pixels |

## Settings

| Setting | Key | Does |
|---|---|---|
| General → Thumbnail goes away after | `card_seconds` | 5, 10 or 20 seconds, or Never |
| General → Where to show | `card_side` | The left or right corner |
| General → Thumbnails at once | `max_cards` | 2, 4 or 6; old ones leave as new ones come |
| Screenshots → Show the thumbnail | `after_shot.card` | The card after a screenshot |
| Screenshots → Copy to the clipboard | `after_shot.copy` | Copy the screenshot right away |
| Screenshots → Save to the folder | `after_shot.save` | Off, the file goes to a temporary folder and is gone after a restart |
| Screenshots → Open the editor | `after_shot.edit` | Annotate right after the capture |
| Screenshots → Pin on top | `after_shot.pin` | Every screenshot floats above all windows |
| Screenshots → Screenshots folder | `screenshot_dir` | Where screenshots go |
| Screenshots → Window shadow | `window_shadow` | A window shot's shadow, on a transparent background |
| Screenshots → Window background | `window_backdrop` | A background saved in the editor, by name, for window shots |
| Screenshots → Screenshot size | `logical_size` | 1× (a pixel of the picture is a pixel of the design) or Screen pixels |
| Screenshots → Pointer in screenshots | `shot_cursor` | The pointer is taken where it was |
| Screenshots → Self-timer | `timer_seconds` | 3, 5 or 10 seconds |

Text recognition's languages are in Settings → Text.

## Commands

```
kadr capture                                   every mode on one bar
kadr shot [smart|region|windows|fullscreen|all|timer|previous]
                                               take a screenshot (smart by default; all: every screen at once;
                                               previous: the last area)
kadr shot scroll ["X,Y WxH"]                    scrolling screenshot (again to finish)
kadr open <file>                               show the thumbnail for an existing image (a .kadr opens its project)
kadr pin <file>                                pin an image on top of windows
kadr text [image]                              recognize text (in an area or an image) and copy it
```
