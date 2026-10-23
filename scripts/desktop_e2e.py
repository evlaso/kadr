#!/usr/bin/env python3
"""The pointer e2e test on macOS and Windows, where there's no nested
Hyprland: open the editor on the real desktop, click the box tool where it
is, drag a box out on the picture, Ctrl+S, and check the file changed. With a
real pointer and keyboard (kadr-pointer, enigo) and screenshots on the way
(kadr-shot, xcap), saved to target/desktop-e2e/<system>-<shell>/ for the CI's
artifacts.

  scripts/desktop_e2e.py gpui|iced

gpui is kadr itself, found and driven through its dev hooks (KADR_DEV=1);
iced is the phase 2 spike (branch spike/iced), found by its window title.
Needs `cargo build --release -p kadr -p kadr-testkit` (and -p kadr-ui-iced).
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


def key(combo):
    if LINUX:
        mods = combo.split("+")[:-1]
        args = sum((["-M", m] for m in mods), []) + [combo.split("+")[-1]] + sum((["-m", m] for m in mods), [])
        run("wtype", *args)
    else:
        run(POINTER, "key", combo)


def run(*args, check=True, timeout=30):
    """A tool, waited for at most `timeout` seconds (a kadr that never
    answers mustn't hang the job)."""
    try:
        return subprocess.run([str(a) for a in args], capture_output=True, text=True, check=check, timeout=timeout)
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

    def finish(self):
        ok = all(s["ok"] for s in self.steps) and bool(self.steps)
        (self.out / "result.json").write_text(json.dumps({"shell": self.shell, "system": platform.platform(), "ok": ok, "steps": self.steps}, indent=2) + "\n")
        return ok


def dev(*args):
    r = run(BIN / f"kadr{EXE}", "dev", *args, check=False, timeout=10)
    v = json.loads(r.stdout)
    return v["result"] if v.get("ok") else None


def gpui(t, work):
    """kadr's editor: the window and the tool's button from the dev hooks,
    which give real screen positions on macOS and Windows."""
    env = dict(os.environ, KADR_DEV="1")
    # Its output away from ours: what it starts (a clipboard helper) may
    # outlive it holding it.
    proc = subprocess.Popen([str(BIN / f"kadr{EXE}"), "edit", str(work)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    win = wait_for(lambda: next((w for w in dev("windows") or [] if w["kind"] == "kadr-editor"), None))
    if not t.note("the editor opens", win):
        return proc
    t.note("the picture loads", wait_for(lambda: (dev("state") or {}).get("view", {}).get("image")))
    time.sleep(1)
    t.shot("opened")
    # Again now that it's settled: the first answer can be the size asked
    # for, before the system gave the window its own.
    win = next((w for w in dev("windows") or [] if w["kind"] == "kadr-editor"), win)
    # Window and element bounds are logical; Windows' pointer is physical.
    (wx, wy, ww, wh), scale = win["bounds"], win["scale"] if os.name == "nt" else 1.0
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
    return proc


def iced(t, work):
    """The iced spike: found by its window's title; its toolbar is at fixed
    places from the window's top left (it has no dev hooks)."""
    exe = BIN / f"kadr-iced{EXE}"
    if not exe.exists():
        t.note("the iced spike is built", False, hint="it lives on branch spike/iced")
        return None
    proc = subprocess.Popen([str(exe), "edit", str(work)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def windows():
        if LINUX:
            clients = json.loads(run("hyprctl", "-j", "clients").stdout)
            return [{"title": c["title"], "x": c["at"][0], "y": c["at"][1], "w": c["size"][0], "h": c["size"][1]} for c in clients]
        return json.loads(run(SHOT, "windows").stdout)

    def window():
        return next((w for w in windows() if w["title"] == "kadr-iced editor"), None)

    win = wait_for(window)
    if not t.note("the editor opens with its picture", win):
        return proc
    time.sleep(1)
    t.shot("opened")
    run(POINTER, "click", win["x"] + 105, win["y"] + 24)
    cx, cy = win["x"] + win["w"] / 2, win["y"] + win["h"] / 2
    run(POINTER, "drag", cx - 100, cy - 60, cx + 100, cy + 60)
    t.shot("drawn")
    return proc


def main():
    shell = sys.argv[1] if len(sys.argv) > 1 else "gpui"
    t = Test(shell)
    work = t.out / "shot.png"
    shutil.copy(ROOT / "tests/golden/cases/arrow/base.png", work)
    before = work.read_bytes()
    proc = (gpui if shell == "gpui" else iced)(t, work)
    if proc is not None and all(s["ok"] for s in t.steps):
        key("ctrl+s")
        t.note("Ctrl+S saves a changed file", wait_for(lambda: work.read_bytes() != before, timeout=10))
        t.shot("saved")
    if proc is not None and proc.poll() is None:
        proc.kill()
    ok = t.finish()
    print(f"{shell}: {'passed' if ok else 'FAILED'} ({t.out})")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
