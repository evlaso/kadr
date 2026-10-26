#!/usr/bin/env python3
"""The pointer e2e test on macOS and Windows, where there's no nested
Hyprland: open the editor on the real desktop, click the box tool where it
is, drag a box out on the picture, type with the system's text keys, pick
from the system's menu, Ctrl+S, and take an area through the overlay. With
a real pointer and keyboard (kadr-pointer, enigo) and screenshots on the
way (kadr-shot, xcap), saved to target/desktop-e2e/<system>-gpui/ for the
CI's artifacts. kadr is found and driven through its dev hooks (KADR_DEV=1).

  scripts/desktop_e2e.py

Needs `cargo build --release -p kadr -p kadr-testkit -p kadr-overlay`.

The outcome in result.json is `passed`, `failed`, or `gpu_unavailable`: the
shell's log (app.log) shows it couldn't get a GPU to draw with, which says
the runner has none, not that the framework failed; that one exits 0 too.
`renderer` says what really drew: GPUI's adapter (a software one is WARP on
Windows).
On Linux it runs too, inside `scripts/nested.sh`'s session (grim and wtype
stand in for kadr-shot and the keyboard), to try the script itself.
"""

import json
import os
import platform
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""
BIN = ROOT / "target/release"
POINTER = BIN / f"kadr-pointer{EXE}"
SHOT = BIN / f"kadr-shot{EXE}"
LINUX = platform.system() == "Linux"

# What a shell's log says when it got no GPU to draw with: GPUI on Metal,
# Direct3D 11 (it tries the DXGI adapters, WARP among them when Windows lists
# it, with no WARP device of its own) and wgpu.
GPU_ERRORS = [
    "failed to initialize a gpu context",
    "unable to access a compatible graphics device",
    "no compatible direct3d 11 adapter",
    "getting dxgi adapter",
    "d3d11createdevice",
    "no gpu adapter",
    "gpu resources not available",
    "graphicscreationfailed",
    "graphicsadapternotfound",
    "failed to create wgpu",
]


def key(combo):
    if LINUX:
        mods = combo.split("+")[:-1]
        last = combo.split("+")[-1]
        names = {"backspace": "BackSpace", "left": "Left", "right": "Right", "escape": "Escape", "enter": "Return", "delete": "Delete"}
        press = ["-k", names[last]] if last in names else [last]
        args = sum((["-M", m] for m in mods), []) + press + sum((["-m", m] for m in mods), [])
        run("wtype", *args)
    else:
        run(POINTER, "key", combo)


def run(*args, check=True, timeout=30):
    """A tool, waited for at most `timeout` seconds (a kadr that never
    answers mustn't hang the job)."""
    try:
        # UTF-8 whatever the system's code page (cp1252 on Windows would
        # read "мир" in kadr's JSON as "Ð¼Ð¸Ñ€").
        return subprocess.run([str(a) for a in args], capture_output=True, encoding="utf-8", errors="replace", check=check, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return subprocess.CompletedProcess(e.cmd, 124, "", f"timed out after {timeout} s")


def wait_for(check, timeout=20.0, step=0.1):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            v = check()
        except Exception:
            v = None
        if v:
            return v
        time.sleep(step)
    return None


class Test:
    def __init__(self, shell):
        self.shell = shell
        self.out = ROOT / "target/desktop-e2e" / f"{platform.system().lower()}-{shell}"
        shutil.rmtree(self.out, ignore_errors=True)
        self.out.mkdir(parents=True)
        self.steps = []
        self.shots = 0
        self.log = self.out / "app.log"
        self.renderer = None

    def note(self, step, ok, **detail):
        self.steps.append({"step": step, "ok": bool(ok), **detail})
        print(f"{'ok ' if ok else 'FAIL'} {step} {detail if detail else ''}", flush=True)
        return ok

    def shot(self, name):
        self.shots += 1
        path = self.out / f"{self.shots:02d}-{name}.png"
        r = run("grim", path, check=False) if LINUX else run(SHOT, "screen", path, check=False)
        if r.returncode != 0:
            print(f"  (no screenshot: {r.stderr.strip()})")

    def start(self, *args, env=None):
        """The shell under test, its output in app.log: read for the GPU's
        errors, and away from ours (what it starts, a clipboard helper, may
        outlive it holding it)."""
        with open(self.log, "ab") as log:
            return subprocess.Popen([str(a) for a in args], env=env, stdout=log, stderr=log)

    def gpu_error(self):
        try:
            lines = self.log.read_text(errors="replace").splitlines()
        except OSError:
            return None
        return next((l.strip() for l in lines if any(e in l.lower() for e in GPU_ERRORS)), None)

    def finish(self):
        ok = all(s["ok"] for s in self.steps) and bool(self.steps)
        gpu = None if ok else self.gpu_error()
        outcome = "passed" if ok else "gpu_unavailable" if gpu else "failed"
        result = {"shell": self.shell, "system": platform.platform(), "outcome": outcome, "ok": ok, "renderer": self.renderer, "steps": self.steps}
        if gpu:
            result["gpu_error"] = gpu
        (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        return outcome


def screen():
    """The first monitor as (x, y, w, h)."""
    if LINUX:
        m = json.loads(run("hyprctl", "-j", "monitors").stdout)[0]
        return (m["x"], m["y"], m["width"], m["height"])
    m = json.loads(run(SHOT, "monitor").stdout)
    return (m["x"], m["y"], m["w"], m["h"])


def fits(t, x, y, w, h):
    """The window is wholly on the screen."""
    sx, sy, sw, sh = screen()
    return t.note("the window fits the screen", sx <= x and sy <= y and x + w <= sx + sw and y + h <= sy + sh, window=[x, y, w, h], screen=[sx, sy, sw, sh])


def pixel(x, y):
    """The screen's colour at a point, as (r, g, b)."""
    try:
        if LINUX:
            out = subprocess.run(["grim", "-g", f"{int(x)},{int(y)} 1x1", "-t", "ppm", "-"], capture_output=True, timeout=10).stdout
        else:
            out = bytes.fromhex(run(SHOT, "pixel", x, y, check=False).stdout.strip().lstrip("#"))
    except (subprocess.TimeoutExpired, ValueError):
        out = b""
    return tuple(out[-3:]) if len(out) >= 3 else (0, 0, 0)


def dev(*args):
    r = run(BIN / f"kadr{EXE}", "dev", *args, check=False, timeout=10)
    v = json.loads(r.stdout)
    return v["result"] if v.get("ok") else None


def gpui(t, work):
    """kadr's editor: the window and the tool's button from the dev hooks,
    which give real screen positions on macOS and Windows."""
    # KADR_DEV also puts GPUI's log (the GPU it took) on stderr.
    # Screenshots land in the test's folder (the overlay step below).
    t0 = time.monotonic()
    proc = t.start(BIN / f"kadr{EXE}", "edit", work, env=dict(os.environ, KADR_DEV="1", OMARCHY_SCREENSHOT_DIR=str(t.out / "shots"), OMARCHY_SCREENRECORD_DIR=str(t.out / "videos")))
    win = wait_for(lambda: proc.poll() is None and next((w for w in dev("windows") or [] if w["kind"] == "kadr-editor"), None))
    # A cold start: the process to the editor's window, and kadr's own
    # steps ("kadr start: step=ms" in its log).
    opened = round((time.monotonic() - t0) * 1000)
    time.sleep(0.3)
    log = t.log.read_text(encoding="utf-8", errors="replace").replace("\0", "") if t.log.exists() else ""
    steps = {k: v for line in log.splitlines() if line.startswith("kadr start: ") for k, _, v in [line[12:].partition("=")]}
    if not t.note("the editor opens", win, ms=opened, **steps):
        return proc
    t.note("the picture loads", wait_for(lambda: (dev("state") or {}).get("view", {}).get("image")))
    time.sleep(1)
    t.shot("opened")
    # Again now that it's settled: the first answer can be the size asked
    # for, before the system gave the window its own.
    win = next((w for w in dev("windows") or [] if w["kind"] == "kadr-editor"), win)
    gpu = win.get("gpu") or {}
    t.renderer = {"framework": "gpui", **gpu}
    if os.name == "nt":
        # GPUI's software flag stays false for WARP, which Windows lists as
        # the Microsoft Basic Render Driver.
        t.renderer["warp"] = bool(gpu.get("software")) or "Basic Render Driver" in (gpu.get("device") or "")
    if not gpu and platform.system() == "Darwin":
        t.renderer["note"] = "Metal; GPUI reports no adapter on macOS"
    # Window and element bounds are logical; Windows' pointer is physical.
    (wx, wy, ww, wh), scale = win["bounds"], win["scale"] if os.name == "nt" else 1.0
    if LINUX:
        # Wayland doesn't tell a window where it is; Hyprland does.
        c = next((c for c in json.loads(run("hyprctl", "-j", "clients").stdout) if c["class"] == "kadr-editor"), None)
        if c:
            wx, wy = c["at"]
    fits(t, wx * scale, wy * scale, ww * scale, wh * scale)
    tool = dev("find", "tool-r")
    if not t.note("the box tool is found", tool):
        return proc
    b = tool["bounds"]
    screen = lambda x, y: ((wx + x) * scale, (wy + y) * scale)
    run(POINTER, "click", *screen(b[0] + b[2] / 2, b[1] + b[3] / 2))
    t.note("the box tool is picked", wait_for(lambda: (dev("state") or {}).get("view", {}).get("tool") == "rect", timeout=5), tool=(dev("state") or {}).get("view", {}).get("tool"))
    cx, cy = ww / 2, wh / 2
    run(POINTER, "drag", *screen(cx - 100, cy - 60), *screen(cx + 100, cy + 60))
    shapes = wait_for(lambda: (dev("state") or {}).get("view", {}).get("shapes"), timeout=5)
    t.note("a box is drawn", shapes and shapes[0]["kind"] == "rect", shapes=len(shapes or []))
    t.shot("drawn")
    typing(t, screen, b, ww, wh)
    menu(t, screen)
    return proc


def menu(t, screen):
    """A dropdown opens the system's menu (macOS, Windows): the zoom's,
    clicked for real, 25% picked with the keyboard as in any system menu
    (Down onto Fit, Down onto 25%, Enter). Linux draws its own."""
    if LINUX:
        return
    zoom = dev("find", "dropdown-zoom")
    if not t.note("the zoom dropdown is found", zoom):
        return
    z = zoom["bounds"]
    run(POINTER, "click", *screen(z[0] + z[2] / 2, z[1] + z[3] / 2))
    time.sleep(0.8)
    t.shot("menu")
    for k in ["down", "down", "enter"]:
        key(k)
        time.sleep(0.2)
    picked = wait_for(lambda: abs(((dev("state") or {}).get("view", {}).get("zoom") or 0) - 0.25) < 0.01, timeout=5)
    t.note("the system's menu picks", picked, zoom=(dev("state") or {}).get("view", {}).get("zoom"))


def typing(t, screen, tool, ww, wh):
    """The text tool with the system's text keys: Unicode typed in, a word
    deleted with Option+Backspace (macOS) or Ctrl+Backspace, a selection
    made with Shift+Left and typed over."""
    mac = platform.system() == "Darwin"
    text_tool = dev("find", "tool-t")
    if not t.note("the text tool is found", text_tool):
        return
    tb = text_tool["bounds"]
    run(POINTER, "click", *screen(tb[0] + tb[2] / 2, tb[1] + tb[3] / 2))
    run(POINTER, "click", *screen(ww / 2 - 150, wh / 2 + 90))
    started = wait_for(lambda: (dev("state") or {}).get("view", {}).get("typing") is not None, timeout=5)
    if not t.note("a text is started", started):
        return
    typed = lambda: ((dev("state") or {}).get("view", {}).get("typing") or {}).get("text")
    type_text("Hello world")
    t.note("text types", wait_for(lambda: typed() == "Hello world", timeout=5), text=typed())
    key("alt+backspace" if mac else "ctrl+backspace")
    t.note("a word goes with the system's key", wait_for(lambda: typed() == "Hello ", timeout=5), text=typed())
    type_text("мир ✓")
    t.note("Cyrillic and a symbol type", wait_for(lambda: typed() == "Hello мир ✓", timeout=5), text=typed())
    key("shift+left")
    key("shift+left")
    type_text("!")
    t.note("typing replaces the selection", wait_for(lambda: typed() == "Hello мир!", timeout=5), text=typed())
    t.shot("typed")
    key("escape")


def overlay(t, warm=False):
    """The area picker, kadr-overlay's own window on macOS and Windows:
    shown by `kadr shot region`, a box dragged with the real pointer, the
    screenshot saved at that size. `warm`: through the overlay kept running
    (`kadr-overlay --serve`, as the tray does on macOS)."""
    label = {False: "", True: " (warm)", "again": " (warm, again)"}[warm]
    # Where kadr saves (its OMARCHY_SCREENSHOT_DIR); this pick's is the new one.
    shots = t.out / "shots"
    shots.mkdir(exist_ok=True)
    before = set(shots.glob("*.png"))
    t0 = time.monotonic()
    # Its output in app.log: it may be the kadr that serves the rest.
    t.start(BIN / f"kadr{EXE}", "shot", "region")
    found = wait_for(lambda: next((w for w in json.loads(run(SHOT, "windows").stdout) if w["title"] == "kadr-picker"), None), timeout=10)
    if not t.note("the overlay shows" + label, found, ms=round((time.monotonic() - t0) * 1000), window=found and [found["x"], found["y"], found["w"], found["h"]]):
        return
    time.sleep(0.5)
    t.shot("overlay" + label.replace(" (", "-").replace(", ", "-").rstrip(")"))
    sx, sy, sw, sh = screen()
    cx, cy = sx + sw / 2, sy + sh / 2
    run(POINTER, "drag", cx - 200, cy - 120, cx + 200, cy + 120)
    saved = wait_for(lambda: next(iter(sorted(set(shots.glob("*.png")) - before)), None), timeout=10)
    size = None
    if saved:
        # The PNG's own size, from its header.
        head = saved.read_bytes()[16:24]
        size = [int.from_bytes(head[:4], "big"), int.from_bytes(head[4:], "big")]
    t.note("the area is saved" + label, saved and size and size[0] >= 390 and size[1] >= 230, size=size)
    # Where the time went: kadr's grab and files, the program's own steps,
    # in kadr's log once the overlay ended.
    time.sleep(0.3)
    timing = {}
    logs = [t.log] + ([t.out / "overlay-serve.log"] if warm else [])
    for log in logs:
        if not log.exists():
            continue
        for line in log.read_text(encoding="utf-8", errors="replace").splitlines():
            if "overlay timing:" in line:
                for part in line.split(":", 1)[1].split():
                    k, _, v = part.partition("=")
                    timing[k] = v
    t.note("the overlay's timings are logged" + label, timing, **timing)


def mp4_duration(path):
    """Seconds from an MP4's movie header (moov/mvhd), None while it has none
    (still being written)."""
    data = path.read_bytes()

    def boxes(start, end):
        while start + 8 <= end:
            size, kind = int.from_bytes(data[start:start + 4], "big"), data[start + 4:start + 8]
            if size == 1:
                size = int.from_bytes(data[start + 8:start + 16], "big")
            if size < 8 or start + size > end:
                return
            yield kind, start + 8, start + size
            start += size

    for kind, body, end in boxes(0, len(data)):
        if kind == b"moov":
            for kind, b, _ in boxes(body, end):
                if kind == b"mvhd":
                    v1 = data[b] == 1
                    scale_at = b + (20 if v1 else 12)
                    scale = int.from_bytes(data[scale_at:scale_at + 4], "big")
                    length = int.from_bytes(data[scale_at + 4:scale_at + (12 if v1 else 8)], "big")
                    return length / scale if scale else None
    return None


def mp4_frames(path):
    """The first track's sample count (the video's frames), from its stsz box."""
    data = path.read_bytes()
    at = data.find(b"stsz")
    return int.from_bytes(data[at + 12:at + 16], "big") if at >= 0 else None


def record(t):
    """A recording through the system's recorder (kadr_platform::record):
    `kadr record fullscreen`, about three seconds, `kadr record stop`, the
    MP4's length from its header."""
    videos = t.out / "videos"
    videos.mkdir(exist_ok=True)
    before = set(videos.glob("*.mp4"))
    t0 = time.monotonic()
    run(BIN / f"kadr{EXE}", "record", "fullscreen", check=False)
    file = wait_for(lambda: next(iter(set(videos.glob("*.mp4")) - before), None), timeout=15)
    if not t.note("recording starts", file, ms=round((time.monotonic() - t0) * 1000)):
        return
    time.sleep(3)
    t.shot("recording")
    run(BIN / f"kadr{EXE}", "record", "stop", check=False)
    length = wait_for(lambda: mp4_duration(file), timeout=20)
    # The recorder's own log beside the test's, whatever came of it.
    caches = Path(os.environ.get("LOCALAPPDATA", "")) / "kadr/cache" if os.name == "nt" else Path.home() / "Library/Caches/kadr"
    if (caches / "record.log").exists():
        shutil.copy(caches / "record.log", t.out / "record.log")
    frames = length and mp4_frames(file)
    t.note("the recording is saved", length and 2 <= length <= 8, seconds=length and round(length, 2), size=file.stat().st_size if file.exists() else None)
    # A steady frame rate, like gpu-screen-recorder's on Linux, even with
    # nothing moving on screen: at least 15 frames a second.
    t.note("the recording has a steady frame rate", frames and frames >= 15 * length, frames=frames, fps=frames and round(frames / length, 1))


def png_size(path):
    head = path.read_bytes()[16:24]
    return [int.from_bytes(head[:4], "big"), int.from_bytes(head[4:], "big")]


def editor_window(t):
    """A window of kadr's to take: the editor, opened again if saving
    closed it."""
    find = lambda: next((w for w in dev("windows") or [] if w["kind"] == "kadr-editor"), None)
    if find() is None:
        env = dict(os.environ, KADR_DEV="1", OMARCHY_SCREENSHOT_DIR=str(t.out / "shots"), OMARCHY_SCREENRECORD_DIR=str(t.out / "videos"))
        t.start(BIN / f"kadr{EXE}", "edit", t.out / "shot.png", env=env)
        wait_for(find, timeout=20)
        time.sleep(1)
    return find()


def window_shot(t):
    """A click on the editor's window in the overlay (`kadr shot smart`,
    windows to snap to from the system): the window taken by itself, with
    kadr's shadow around it."""
    shots = t.out / "shots"
    before = set(shots.glob("*.png"))
    win = editor_window(t)
    if not t.note("the editor's window is there to take", win):
        return
    (wx, wy, ww, wh), scale = win["bounds"], win["scale"] if os.name == "nt" else 1.0
    t.log.write_text("")
    t.start(BIN / f"kadr{EXE}", "shot", "smart")
    shown = wait_for(lambda: next((w for w in json.loads(run(SHOT, "windows").stdout) if w["title"] == "kadr-picker"), None), timeout=10)
    if not t.note("the overlay shows for a window", shown):
        return
    time.sleep(0.5)
    cx, cy = (wx + ww / 2) * scale, (wy + wh / 2) * scale
    run(POINTER, "move", cx, cy)
    time.sleep(0.3)
    t.shot("window-hover")
    run(POINTER, "click", cx, cy)
    saved = wait_for(lambda: next(iter(sorted(set(shots.glob("*.png")) - before)), None), timeout=10)
    size = saved and png_size(saved)
    time.sleep(0.3)
    log = t.log.read_text(encoding="utf-8", errors="replace") if t.log.exists() else ""
    how = "by itself" if "window shot by itself" in log else "cut from the screen" if "window shot cut" in log else None
    # The window and kadr's shadow margin (68 logical pixels each side),
    # saved at 1x (the logical_size setting, on by default).
    want = [round(ww + 136), round(wh + 136)]
    near = size and all(abs(a - b) <= 8 for a, b in zip(size, want))
    t.note("the window is taken by itself", near and how == "by itself", size=size, expected=want, how=how)


def png_pixels(path):
    """An 8-bit RGB or RGBA PNG's size and its pixels as (r, g, b) rows,
    without a library."""
    import zlib
    data = path.read_bytes()
    at, idat = 8, b""
    w = h = depth = kind = 0
    while at < len(data):
        n, tag = int.from_bytes(data[at:at + 4], "big"), data[at + 4:at + 8]
        body = data[at + 8:at + 8 + n]
        if tag == b"IHDR":
            w, h, depth, kind = int.from_bytes(body[:4], "big"), int.from_bytes(body[4:8], "big"), body[8], body[9]
        elif tag == b"IDAT":
            idat += body
        at += 12 + n
    if depth != 8 or kind not in (2, 6):
        return None
    bpp = 4 if kind == 6 else 3
    raw, stride, rows, prev = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append([tuple(line[x * bpp:x * bpp + 3]) for x in range(w)])
        prev = line
    return w, h, rows


def pointer_in_shots(t):
    """The `shot_cursor` setting: the same fullscreen shot without and with
    it, the pointer kept still; around the pointer only the second differs."""
    import platform as plat
    config = Path.home() / ("Library/Application Support/kadr" if plat.system() == "Darwin" else "AppData/Roaming/kadr")
    if os.name == "nt" and os.environ.get("APPDATA"):
        config = Path(os.environ["APPDATA"]) / "kadr"
    settings = config / "settings.json"
    old = settings.read_text(encoding="utf-8") if settings.exists() else None
    shots = t.out / "shots"
    sx, sy, sw, sh = screen()
    # Over the desktop, away from the editor and the menu bar.
    px, py = sx + sw - 120, sy + sh - 160
    run(POINTER, "move", px, py)
    taken = []
    for on in (False, True):
        current = json.loads(settings.read_text(encoding="utf-8")) if settings.exists() else {}
        current["shot_cursor"] = on
        config.mkdir(parents=True, exist_ok=True)
        settings.write_text(json.dumps(current), encoding="utf-8")
        before = set(shots.glob("*.png"))
        run(BIN / f"kadr{EXE}", "shot", "fullscreen", check=False)
        taken.append(wait_for(lambda: next(iter(sorted(set(shots.glob("*.png")) - before)), None), timeout=15))
        time.sleep(0.5)
    if old is None:
        settings.unlink(missing_ok=True)
    else:
        settings.write_text(old, encoding="utf-8")
    if not t.note("both shots are taken", all(taken)):
        return
    off, on = png_pixels(taken[0]), png_pixels(taken[1])
    if not off or not on or off[:2] != on[:2]:
        return t.note("the pointer is in the shot", False, sizes=[off and off[:2], on and on[:2]])
    w, h, a = off
    b = on[2]
    # The shot is 1x: the pointer's logical position is its pixel.
    x0, y0 = int(px - sx), int(py - sy)
    near = sum(1 for y in range(max(0, y0 - 4), min(h, y0 + 30)) for x in range(max(0, x0 - 4), min(w, x0 + 24)) if sum(abs(p - q) for p, q in zip(a[y][x], b[y][x])) > 60)
    # Beside it, clear of the cursor and of anything else that moves (the
    # screen's corners have the clock, the taskbar, the menu bar).
    far = sum(1 for y in range(max(0, y0 - 4), min(h, y0 + 30)) for x in range(max(0, x0 - 90), max(0, x0 - 50)) if a[y][x] != b[y][x])
    t.note("the pointer is in the shot", near > 30 and far == 0, changed_near_pointer=near, changed_far_away=far)


def window_capture_probe(t):
    """Does taking a window by itself need the owner's main thread? kadr's
    own window and another app's (Finder, Explorer), each taken on a
    background thread with kadr's main thread free and then blocked
    (`kadr dev grab-window ID [block]`)."""
    other_app = "Finder" if platform.system() == "Darwin" else "explorer"
    editor_window(t)
    if platform.system() == "Darwin":
        run("open", Path.home(), check=False)
    else:
        run("explorer.exe", Path.home(), check=False)
    listed = lambda: json.loads(run(SHOT, "windows").stdout)
    other = wait_for(lambda: next((w for w in listed() if other_app.lower() in (w["app"] or "").lower() and w["w"] > 100), None), timeout=10)
    own = next((w for w in listed() if w["title"] == "kadr" and w["w"] > 300), None)
    if not t.note("windows to probe", own and other, own=own and own["id"], other=other and other["app"]):
        return
    results = {}
    for name, w in (("own", own), ("other", other)):
        for block in (False, True):
            answer = dev("grab-window", w["id"], *(["block"] if block else []))
            results[f"{name}{'-blocked' if block else ''}"] = answer and {k: answer.get(k) for k in ("came", "ms", "size")}
    # The other app's window goes, so it doesn't cover the editor later.
    if platform.system() == "Darwin":
        run("osascript", "-e", 'tell application "Finder" to close every window', check=False)
    else:
        run("powershell", "-NoProfile", "-Command", "(New-Object -ComObject Shell.Application).Windows() | ForEach-Object { $_.Quit() }", check=False)
    # Kept for the record whatever it shows: a deadlock is own-blocked not
    # coming while the others do.
    t.note("window capture probe", all(r and r["came"] for k, r in results.items() if k != "own-blocked"), **results)


def all_screens(t):
    """`kadr shot all`: every screen in one picture, as big as the desktop."""
    shots = t.out / "shots"
    before = set(shots.glob("*.png"))
    run(BIN / f"kadr{EXE}", "shot", "all", check=False)
    saved = wait_for(lambda: next(iter(sorted(set(shots.glob("*.png")) - before)), None), timeout=15)
    _, _, sw, sh = screen()
    size = saved and png_size(saved)
    t.note("every screen is taken at once", size and size[0] >= sw and size[1] >= sh, size=size, screen=[sw, sh])


def type_text(s):
    if LINUX:
        run("wtype", s)
    else:
        # In hex: the runner's shell may hand non-ASCII arguments over in
        # its ANSI code page.
        run(POINTER, "type-hex", s.encode().hex())


def main():
    # UTF-8 out whatever the console's code page (cp1252 on Windows can't
    # print "мир").
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    shell = "gpui"
    if platform.system() == "Darwin":
        # The tray kadr starts keeps no overlay warm here (keep_warm off for
        # the run): the first pick is the cold path, measured on its own.
        os.environ["KADR_KEEP_WARM"] = "0"
    if LINUX:
        # The runners' screen, 1024x768, on the nested session's output.
        run("hyprctl", "eval", 'hl.monitor({ output = "WAYLAND-1", mode = "1024x768@60", position = "0x0", scale = 1 })')
        time.sleep(0.5)
    t = Test(shell)
    work = t.out / "shot.png"
    shutil.copy(ROOT / "tests/golden/cases/arrow/base.png", work)
    before = work.read_bytes()
    proc = gpui(t, work)
    if proc is not None and all(s["ok"] for s in t.steps):
        key("ctrl+s")
        t.note("Ctrl+S saves a changed file", wait_for(lambda: work.read_bytes() != before, timeout=10))
        t.shot("saved")
        if shell == "gpui" and not LINUX:
            # The cold path: a kadr-overlay process for the pick.
            overlay(t)
            if platform.system() == "Darwin":
                # Again, through an overlay of the test's own on the tray's
                # socket, kept running as keep_warm does: it freezes the
                # screen itself, from the first pick on.
                import tempfile
                socket = Path(tempfile.gettempdir()) / "kadr-overlay.sock"
                with open(t.out / "overlay-serve.log", "ab") as log:
                    serve = subprocess.Popen([str(BIN / "kadr-overlay"), "--serve", str(socket)], stdout=log, stderr=log)
                wait_for(lambda: socket.exists(), timeout=10)
                time.sleep(1)
                # Only this run's lines count.
                t.log.write_text("")
                overlay(t, warm=True)
                t.log.write_text("")
                overlay(t, warm="again")
                serve.kill()
            # Before the window shot, which hung on macOS: the probe's
            # numbers say whether capturing kadr's own window needs its
            # main thread.
            window_capture_probe(t)
            window_shot(t)
            all_screens(t)
            pointer_in_shots(t)
            record(t)
    if proc is not None and proc.poll() is None:
        proc.kill()
    outcome = t.finish()
    print(f"{shell}: {outcome} ({t.out}), renderer {t.renderer}")
    return 1 if outcome == "failed" else 0


if __name__ == "__main__":
    sys.exit(main())
