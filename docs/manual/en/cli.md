# Command line

Run `kadr --help` to see the command list. If kadr is already running, later capture, editing and settings commands go to that instance instead of starting another UI process.

```text
  kadr capture                                   every mode on one bar
  kadr shot [smart|region|windows|fullscreen|all|timer|previous]
                                                 take a screenshot (smart by default; all: every screen at once;
                                                 previous: the last area)
  kadr shot scroll ["X,Y WxH"]                  scrolling screenshot (again to finish)
  kadr open <file>                               show the thumbnail for an existing image (a .kadr opens its project)
  kadr pin <file>                                pin an image on top of windows

  kadr record                                    the recording bar (or stop a running recording)
  kadr record region|fullscreen [--mic] [--audio] [--webcam] [--keys] [--studio]
                                                 start recording at once (again to stop)
  kadr record area "X,Y WxH"                   the recording bar for a given area
  kadr record previous                           the recording bar for the last area picked
  kadr record stop                               stop recording
  kadr studio <video|project.kadr> [--render]    studio: zooms, background, clicks and keys (--render exports at once)
  kadr edit <image|project.kadr>                 editor: arrows, text, highlighter, blur…
  kadr trim <video> [--from S --to S]            trim a recording (no flags: the trim window)
  kadr gif <video>                               save a recording as GIF (720px, 15 fps)
  kadr history                                   history of screenshots and recordings
  kadr settings                                  settings
  kadr update                                    look for a newer release now
  kadr tray                                      tray icon (for autostart)
  kadr dev <command>                             test hooks of a kadr started with KADR_DEV=1
                                                 (windows, dump, find, click, drag, key, type, state, shot, mem)
  kadr text [image]                              recognize text (in an area or an image) and copy it
  kadr install-desktop                           let .kadr project files open in kadr (file type, icon, menu entry)
  kadr --version                                 the version
```

## Screenshots

`kadr capture` opens the bar with every capture mode. `kadr shot` starts smart selection by default; choose an area, windows, a screen, every screen, the self-timer or the previous area explicitly. `kadr shot scroll` starts a scrolling screenshot and finishes it when called again. `kadr open <file>` shows an existing image on a card; `kadr pin <file>` keeps it above other windows. See [Capture](capture.md).

## Recording

`kadr record` opens the recording bar, or stops the current recording. The region and fullscreen forms start immediately, with optional microphone, system audio, camera, shortcut display and studio mode. The area and previous forms open the bar for a given or last-picked area. `kadr record stop` stops explicitly. See [Recording](record.md); the Hyprland click/key log is Linux-only.

## Editing

`kadr edit <image|project.kadr>` opens the [editor](editor.md); `kadr studio <video|project.kadr>` opens the [studio](studio.md), and its render flag exports immediately. `kadr open <file>` opens a project in the right editor. `kadr trim <video>` opens trimming, or trims directly with times in seconds; a start time requires an end time. `kadr gif <video>` makes a 720px, 15 fps GIF. See [Project files](projects.md) for keeping an edit.

## Other commands

Open [History](history.md) with `kadr history` or [Settings](settings.md) with `kadr settings`. `kadr text [image]` recognizes text in an image, or asks for an area, and copies it. `kadr update` checks for a release now; `kadr tray` runs the tray for autostart. `kadr install-desktop` registers project files on Linux. `kadr --version` prints the version. The development command is for test hooks, not normal capture work.
