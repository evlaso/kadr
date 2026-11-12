# Project files

A `.kadr` project keeps an edit, not just its finished picture or video. You can reopen it later or move it to another computer and keep working.

## How to open

Use `kadr open <file>` to open a project in the editor or studio, according to its contents. `kadr edit <image|project.kadr>` opens a screenshot project; `kadr studio <video|project.kadr>` opens a recording project.

You can also open a `.kadr` file from the file manager once the file type is registered. On Linux kadr registers it on first run; `kadr install-desktop` repeats that setup. The Windows installer registers the association. See [Install](install.md).

## Saving

Use **Project** in the [editor](editor.md) or [studio](studio.md) to save the original picture or recording and its edit together. The screenshot project keeps every shape; the recording project keeps zooms, cuts and trimming along with the other studio choices.

When you open a project, saving it rewrites the same `.kadr` file. In the editor, **Save** also updates that opened project; in the studio, **Export** does so before making the video. Keep a separate copy first if you want to keep an earlier edit.

## Inside the file

A project is a tar archive with the media and editing information together:

- A screenshot project holds the source picture, added pictures, the drawing and a preview. Black boxes permanently remove what they cover from the editable source too.
- A recording project holds the video, its click/key/cursor log, the studio edit, and the separate camera recording and wallpaper when present.

Opening unpacks the project into kadr's cache. You do not need to unpack it yourself or keep its source files beside it.

## Keys

Ctrl, Cmd on macOS.

| Keys | Does |
|---|---|
| Ctrl+S in the editor | Save the picture and update its opened project |
| Ctrl+S in the studio | Export the video and update its opened project |

## Commands

```text
  kadr open <file>                               show the thumbnail for an existing image (a .kadr opens its project)
  kadr studio <video|project.kadr> [--render]    studio: zooms, background, clicks and keys (--render exports at once)
  kadr edit <image|project.kadr>                 editor: arrows, text, highlighter, blur…
  kadr install-desktop                           let .kadr project files open in kadr (file type, icon, menu entry)
```

`kadr install-desktop` is Linux-only.
