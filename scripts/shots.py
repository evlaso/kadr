#!/usr/bin/env python3
"""Draw the manual's sample media and photograph the real UI in nested Hyprland.

Run `cargo build --release`, then `just shots`. Builds and installs nothing.
Needs Hyprland, ImageMagick, ffmpeg, mpv, grim and wtype. All app files,
including HOME, belong to the disposable session. Never captures the desktop.
"""

import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/release/kadr"
OUT = ROOT / "docs/images"


def run(*args, env=None):
    return subprocess.run(list(map(str, args)), env=env, check=True, capture_output=True, text=True, timeout=30).stdout


def wait(check, timeout=15):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = check()
        if value:
            return value
        time.sleep(0.05)
    raise RuntimeError("Timed out waiting for the nested UI")


class Session:
    def __init__(self):
        # Let nested.sh finish its own bounded startup even if interrupted:
        # until it returns, we don't yet know the folder needed for cleanup.
        output = subprocess.run([str(ROOT / "scripts/nested.sh"), "start"], check=True, capture_output=True,
                                text=True, start_new_session=True).stdout
        self.vars = dict(line[7:].split("=", 1) for line in output.splitlines() if line.startswith("export "))
        self.dir = Path(self.vars["KADR_E2E_DIR"])
        self.env = dict(os.environ, **self.vars, HOME=str(self.dir / "home"), TZ="UTC", LC_ALL="C.UTF-8")
        self.processes = []

    def close(self):
        run(ROOT / "scripts/nested.sh", "stop", self.dir)
        for process in self.processes:
            process.wait(timeout=10)

    def start(self, *args):
        with (self.dir / "app.log").open("ab") as log:
            process = subprocess.Popen(list(map(str, args)), env=self.env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        self.processes.append(process)
        return process

    def app(self, *args):
        return self.start(BIN, *args)

    def stop(self, process):
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        (self.dir / "kadr.sock").unlink(missing_ok=True)

    def dev(self, *args, optional=False):
        try:
            with socket.socket(socket.AF_UNIX) as sock:
                sock.settimeout(10)
                sock.connect(str(self.dir / "kadr.sock"))
                sock.sendall((json.dumps(["dev", *map(str, args)]) + "\n").encode())
                reply = json.loads(sock.makefile().readline())
            if reply["ok"]:
                return reply["result"]
            if not optional:
                raise RuntimeError(f"dev {args}: {reply}")
        except (OSError, ValueError):
            if not optional:
                raise
        return None

    def window(self, kind):
        return wait(lambda: next((w for w in self.dev("windows", optional=True) or [] if w["kind"] == kind), None))

    def keys(self, *keys):
        run("wtype", *sum((["-k", key] for key in keys), []), env=self.env)

    def move(self, x, y):
        run("hyprctl", "eval", f"hl.dispatch(hl.dsp.cursor.move({{x = {x}, y = {y}}}))", env=self.env)

    def shot(self, name, theme, window=None, area=None):
        # Wait out entry animations, decoding and hover tips. No clock is shown.
        time.sleep(0.7)
        raw = self.dir / "shot.png"
        if window is not None:
            self.dev("shot", raw, window["id"])
        else:
            run("grim", "-g", area, raw, env=self.env)
        output = OUT / f"{name}-{theme}.webp"
        run("magick", raw, "-strip", "-define", "webp:lossless=true", "-define", "webp:method=6", output)
        if output.stat().st_size > 150_000:
            run("magick", raw, "-strip", "-quality", "92", "-define", "webp:method=6", output)
        if output.stat().st_size > 150_000:
            raise RuntimeError(f"{output.name} exceeds 150 KB")
        print(f"{output.name}: {output.stat().st_size / 1000:.1f} KB", flush=True)


def samples(s):
    """A fictional planning app, drawn from primitives, not captured content."""
    sample = s.dir / "sample.png"
    font = ROOT / "data/fonts/IBMPlexSans-Regular.ttf"
    bold = ROOT / "data/fonts/IBMPlexSans-SemiBold.ttf"
    run("magick", "-size", "960x600", "gradient:#b9d9e9-#d5c6ef",
        "-fill", "#ffffff", "-draw", "roundrectangle 70,55 890,545 16,16",
        "-fill", "#f3f5f9", "-draw", "roundrectangle 70,55 890,105 16,16 rectangle 70,85 890,105",
        "-fill", "#a8b7c8", "-draw", "circle 94,80 99,80 circle 112,80 117,80 circle 130,80 135,80",
        "-font", bold, "-pointsize", "18", "-fill", "#33445c", "-annotate", "+158+87", "Fieldnotes",
        "-pointsize", "30", "-annotate", "+110+158", "A little room for good ideas",
        "-font", font, "-pointsize", "17", "-fill", "#718198", "-annotate", "+110+191", "Plan the next chapter, one small step at a time.",
        "-fill", "#eff4fa", "-draw", "roundrectangle 110,225 348,498 10,10 roundrectangle 362,225 600,498 10,10 roundrectangle 614,225 852,498 10,10",
        "-font", bold, "-pointsize", "17", "-fill", "#536985", "-annotate", "+130+258", "Ideas", "-annotate", "+382+258", "In progress", "-annotate", "+634+258", "Ready to share",
        "-fill", "white", "-draw", "roundrectangle 124,280 334,365 8,8 roundrectangle 124,380 334,465 8,8 roundrectangle 376,280 586,390 8,8 roundrectangle 628,280 838,365 8,8",
        "-font", font, "-pointsize", "16", "-fill", "#33445c", "-annotate", "+140+310", "Sketch a new direction", "-annotate", "+140+410", "Collect inspiration", "-annotate", "+392+310", "Make the first draft", "-annotate", "+644+310", "Share the story",
        "-pointsize", "13", "-fill", "#8696ac", "-annotate", "+140+341", "A fresh perspective", "-annotate", "+140+441", "Colours, shapes, possibilities", "-annotate", "+392+341", "Keep it simple and clear", "-annotate", "+644+341", "Small steps, real progress",
        "-fill", "#4b83c4", "-draw", "roundrectangle 392,356 464,377 6,6", "-fill", "white", "-pointsize", "12", "-annotate", "+403+371", "Design", "-strip", sample)
    # A short moving picture from the drawn frame; no capture or device input.
    video = Path(s.env["HOME"]) / "videos/fieldnotes.mp4"
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-loop", "1", "-i", sample,
        "-vf", "drawbox=x=392:y=356:w=72:h=21:color=0x56a88b:t=3:enable='gte(t,2)'",
        "-t", "8", "-r", "30", "-c:v", "libx264", "-threads", "1", "-pix_fmt", "yuv420p", "-map_metadata", "-1", video)
    # Compositor captures show only this drawn desktop, never the grey test output.
    wallpaper = s.dir / "desktop.png"
    run("magick", "-size", "1920x1080", "xc:#d8dfed", "(", sample, "-resize", "720x450", ")", "-geometry", "+600+450", "-composite", "-strip", wallpaper)
    # Fill the app's wallpaper gallery with drawn specimens before it considers
    # system wallpapers. The gallery takes twelve from the first 24 candidates.
    walls = Path(s.env["HOME"]) / ".local/state/omarchy/current/theme/backgrounds"
    walls.mkdir(parents=True)
    for i in range(24):
        run("magick", "-size", "160x100", f"gradient:#{80+i*4:02x}a9cf-#c5{80+i*4:02x}b9", "-strip", walls / f"sample-{i:02}.png")
    return sample, video, wallpaper


def shoot(s, theme, sample, video, wallpaper):
    config = Path(s.env["XDG_CONFIG_HOME"]) / "kadr"
    config.mkdir(exist_ok=True)
    (config / "editor-frame.json").unlink(missing_ok=True)
    (config / "settings.toml").write_text(f'language = "en"\ntheme = "{theme}"\ncheck_updates = false\ntray = false\nkeep_warm = false\ncard_seconds = 0\n')
    s.move(1900, 20)
    app = s.app("edit", sample)
    editor = s.window("kadr-editor")
    state = wait(lambda: (s.dev("state", editor["id"])["view"] or {}).get("image"))
    view = s.dev("state", editor["id"])["view"]
    origin, scale = view["image_origin"], view["image_scale"]

    def draw(tool, x, y, dx=0, dy=0):
        s.dev("click", f"tool-{tool}", editor["id"])
        s.dev("drag", f"{origin[0] + x * scale},{origin[1] + y * scale}", f"{dx * scale},{dy * scale}", editor["id"])

    draw("r", 370, 274, 222, 122)
    draw("a", 535, 205, -45, 67)
    draw("t", 540, 182)
    s.dev("type", "Start here", "--window", editor["id"])
    s.dev("key", "escape", "--window", editor["id"])
    draw("n", 353, 277)
    s.dev("key", "escape", "--window", editor["id"])
    s.shot("editor", theme, editor)
    s.dev("click", "mode-background", editor["id"])
    s.dev("click", "gradient-1", editor["id"])
    s.shot("editor-background", theme, editor)
    s.stop(app)

    tracks = Path(s.env["XDG_CACHE_HOME"]) / "kadr/tracks"
    tracks.mkdir(parents=True, exist_ok=True)
    (tracks / "fieldnotes.mp4.json").write_text(json.dumps({"rect": [0, 0, 960, 600], "cursor_hidden": True, "cursor": [[0, 460, 320], [8, 500, 340]], "clicks": [[2, 480, 320]], "keys": [], "keystrokes": [], "shapes": [[0, "arrow"]]}))
    (tracks / "fieldnotes.mp4.json.studio").unlink(missing_ok=True)
    app = s.app("studio", video)
    studio = s.window("kadr-studio")
    # Select the automatic zoom, but show the whole frame at a fixed playhead.
    height = studio["bounds"][3]
    s.dev("drag", f"450,{height - 66}", "0,0", studio["id"])
    s.dev("key", "home", "--window", studio["id"])

    def decoded():
        # A seek made before scrub decoding finishes has no quick frame yet.
        # Retrying a fixed position once the strip is ready is deterministic.
        s.dev("key", "home", "--window", studio["id"])
        time.sleep(0.5)
        raw = s.dir / "ready.png"
        s.dev("shot", raw, studio["id"])
        # Both the stage and the exact filmstrip must have their drawn frames.
        return all(float(run("magick", raw, "-crop", area, "+repage", "-format", "%[fx:mean]", "info:")) > 0.5
                   for area in ("100x100+700+200", f"100x30+600+{int(height - 130)}"))

    wait(decoded, timeout=30)
    s.shot("studio", theme, studio)
    s.stop(app)

    shots = Path(s.env["OMARCHY_SCREENSHOT_DIR"])
    for i in range(4):
        item = shots / f"screenshot-2024-06-{12 + i}_10-30-00.png"
        run("magick", sample, "-modulate", f"100,100,{100 + i * 6}", "-strip", item)
        os.utime(item, (1718188200 + i * 86400,) * 2)
    os.utime(video, (1718188200,) * 2)
    app = s.app("history")
    history = s.window("kadr-history")
    time.sleep(2)
    s.shot("history", theme, history)
    s.stop(app)

    app = s.app("settings")
    s.shot("settings", theme, s.window("kadr-settings"))
    s.stop(app)

    desktop = s.start("mpv", "--no-config", "--vo=gpu", "--gpu-context=wayland", "--fullscreen", "--no-osc", "--no-border", "--cursor-autohide=always", "--image-display-duration=inf", wallpaper)
    time.sleep(1)
    app = s.app("open", sample)
    card = s.window("kadr")
    time.sleep(0.5)
    s.move(125, 970)
    s.shot("card", theme, area="24,896 213x160")
    s.stop(app)
    s.move(1900, 20)

    app = s.app("capture")
    s.window("kadr-capture")
    s.shot("capture-bar", theme, area="480,810 960x200")
    s.stop(app)

    app = s.app("record", "area", "560,450 800x450")
    s.window("kadr-panel")
    s.shot("record", theme, area="515,420 890x590")
    s.stop(app)

    (config / "last-area.json").write_text(json.dumps({"monitor": "WAYLAND-1", "area": [560, 450, 800, 450]}))
    app = s.app("shot", "region")
    wait(lambda: '"kadr-picker"' in run("hyprctl", "-j", "layers", env=s.env))
    time.sleep(0.5)
    # Previous area is a normal picker action and leaves a selection to adjust.
    # Set its input in this session only; no pointer drag can accidentally take it.
    s.keys("p")
    s.move(1900, 20)
    s.shot("capture", theme, area="480,400 960x660")
    s.keys("Escape")
    s.stop(app)
    s.stop(desktop)


def main():
    if not BIN.is_file():
        raise SystemExit("Build first: cargo build --release")
    # SIGTERM and Ctrl+C take the same cleanup path as a failed assertion.
    # Defer them during startup (until its directory is known) and cleanup.
    s, interrupted, closing = None, False, False

    def interrupt(*_):
        nonlocal interrupted
        if s is None or closing:
            interrupted = True
        else:
            raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGINT, interrupt)
    OUT.mkdir(exist_ok=True)
    try:
        s = Session()
        if interrupted:
            raise KeyboardInterrupt
        sample, video, wallpaper = samples(s)
        for theme in ("dark", "light"):
            shoot(s, theme, sample, video, wallpaper)
        for image in sorted(OUT.glob("*.webp")):
            print(hashlib.sha256(image.read_bytes()).hexdigest(), image.name)
    finally:
        closing = True
        if s is not None:
            s.close()


if __name__ == "__main__":
    main()
