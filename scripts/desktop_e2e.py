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
    proc = t.start(BIN / f"kadr{EXE}", "edit", work, env=dict(os.environ, KADR_DEV="1", OMARCHY_SCREENSHOT_DIR=str(t.out / "shots")))
    win = wait_for(lambda: proc.poll() is None and next((w for w in dev("windows") or [] if w["kind"] == "kadr-editor"), None))
    if not t.note("the editor opens", win):
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
    label = " (warm)" if warm else ""
    shots = t.out / ("shots-warm" if warm else "shots")
    shots.mkdir(exist_ok=True)
    t0 = time.monotonic()
    subprocess.Popen([str(BIN / f"kadr{EXE}"), "shot", "region"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    found = wait_for(lambda: next((w for w in json.loads(run(SHOT, "windows").stdout) if w["title"] == "kadr-picker"), None), timeout=10)
    if not t.note("the overlay shows" + label, found, ms=round((time.monotonic() - t0) * 1000), window=found and [found["x"], found["y"], found["w"], found["h"]]):
        return
    time.sleep(0.5)
    t.shot("overlay-warm" if warm else "overlay")
    sx, sy, sw, sh = screen()
    cx, cy = sx + sw / 2, sy + sh / 2
    run(POINTER, "drag", cx - 200, cy - 120, cx + 200, cy + 120)
    saved = wait_for(lambda: next(iter(sorted(shots.glob("*.png"))), None), timeout=10)
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
            overlay(t)
            if platform.system() == "Darwin":
                # Again, warm: the overlay kept running on the tray's socket.
                import tempfile
                socket = Path(tempfile.gettempdir()) / "kadr-overlay.sock"
                with open(t.out / "overlay-serve.log", "ab") as log:
                    serve = subprocess.Popen([str(BIN / "kadr-overlay"), "--serve", str(socket)], stdout=log, stderr=log)
                wait_for(lambda: socket.exists(), timeout=10)
                time.sleep(1)
                # Only this run's lines count.
                t.log.write_text("")
                overlay(t, warm=True)
                serve.kill()
    if proc is not None and proc.poll() is None:
        proc.kill()
    outcome = t.finish()
    print(f"{shell}: {outcome} ({t.out}), renderer {t.renderer}")
    return 1 if outcome == "failed" else 0


if __name__ == "__main__":
    sys.exit(main())
