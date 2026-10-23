#!/usr/bin/env python3
"""Phase 2 (docs/SPEC.ru.md §9.3): the same measurements and the same checks
for each UI shell, in a nested Hyprland of its own (scripts/nested.sh), so
the GPUI app and the spikes compare like for like.

  scripts/spike.py gpui|iced|tauri [--runs N] [--no-build]

Measures: edit → window with the picture, the editor's RSS and idle CPU with
a 4K picture, the dist binary's size, an incremental --release build. Checks,
with a real pointer (kadr-pointer) and keyboard (wtype): an arrow and a box
drawn, text typed, Ctrl+S saves a changed file. Writes target/spike-<shell>.json.
"""

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
TICK = os.sysconf("SC_CLK_TCK")
POINTER = ROOT / "target/release/kadr-pointer"

# Per shell: the package, its binary, how to open the editor, and the
# file to touch for the incremental build.
SHELLS = {
    "gpui": {"package": "kadr", "bin": "kadr", "edit": ["edit"], "touch": "src/ui.rs"},
    "iced": {"package": "kadr-ui-iced", "bin": "kadr-iced", "edit": ["edit"], "touch": "crates/kadr-ui-iced/src/main.rs"},
    "tauri": {"package": "kadr-ui-tauri", "bin": "kadr-tauri", "edit": ["edit"], "touch": "crates/kadr-ui-tauri/src/main.rs"},
}

SESSION = {}


def env():
    return dict(os.environ, **SESSION)


def start_session():
    out = subprocess.run([str(ROOT / "scripts/nested.sh"), "start"], check=True, capture_output=True, text=True).stdout
    for line in out.splitlines():
        if line.startswith("export ") and "=" in line:
            k, v = line[len("export "):].split("=", 1)
            SESSION[k] = v


def stop_session():
    if "KADR_E2E_DIR" in SESSION:
        subprocess.run([str(ROOT / "scripts/nested.sh"), "stop", SESSION["KADR_E2E_DIR"]], check=False)


def hypr(request):
    out = subprocess.run(["hyprctl", "-j", request], env=env(), capture_output=True, text=True).stdout
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return []


def wait_for(check, timeout=10.0, step=0.005):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        v = check()
        if v:
            return v
        time.sleep(step)
    return None


def session_pids(name):
    mark = f"XDG_RUNTIME_DIR={SESSION['XDG_RUNTIME_DIR']}".encode()
    pids = []
    for p in Path("/proc").iterdir():
        if not p.name.isdigit():
            continue
        try:
            argv = (p / "cmdline").read_bytes().split(b"\0")
            if Path(argv[0].decode(errors="replace")).name != name or mark not in (p / "environ").read_bytes().split(b"\0"):
                continue
        except OSError:
            continue
        if len(argv) > 1 and argv[1] in (b"tray", b"dev"):
            continue
        pids.append(int(p.name))
    return pids


def stop_all(name):
    for pid in session_pids(name):
        try:
            os.kill(pid, 15)
        except ProcessLookupError:
            pass
    wait_for(lambda: not session_pids(name), timeout=3)


def tree(pid):
    """The process and everything it started (a webview's own processes)."""
    children = {}
    for p in Path("/proc").iterdir():
        if p.name.isdigit():
            try:
                parent = int((p / "stat").read_text().rsplit(")", 1)[1].split()[1])
            except (OSError, ValueError, IndexError):
                continue
            children.setdefault(parent, []).append(int(p.name))
    out, todo = [], [pid]
    while todo:
        q = todo.pop()
        out.append(q)
        todo.extend(children.get(q, []))
    return out


def rss_mb(pid):
    total = 0
    for q in tree(pid):
        try:
            for line in Path(f"/proc/{q}/status").read_text().splitlines():
                if line.startswith("VmRSS:"):
                    total += int(line.split()[1])
        except OSError:
            pass
    return total / 1024


def cpu_ticks(pid):
    total = 0
    for q in tree(pid):
        try:
            fields = Path(f"/proc/{q}/stat").read_text().rsplit(")", 1)[1].split()
            total += int(fields[11]) + int(fields[12])
        except OSError:
            pass
    return total


def loaded(shell, binary):
    """The editor is up with its picture in."""
    if shell == "gpui":
        out = subprocess.run([str(binary), "dev", "state"], env=env(), capture_output=True, text=True).stdout
        try:
            return bool(json.loads(out)["result"]["view"]["image"])
        except (json.JSONDecodeError, KeyError, TypeError):
            return False
    # The spikes put it in their window's title.
    return any(c.get("title", "").endswith("editor") for c in hypr("clients"))


def open_editor(shell, binary, image):
    stop_all(binary.name)
    t0 = time.monotonic()
    proc = subprocess.Popen([str(binary), *SHELLS[shell]["edit"], image], env=env(), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    ok = wait_for(lambda: loaded(shell, binary), timeout=15)
    return proc, ((time.monotonic() - t0) * 1000 if ok else None)


def pointer(*args):
    subprocess.run([str(POINTER), *map(str, args)], env=env(), check=True)


def keys(*args):
    subprocess.run(["wtype", *args], env=env(), check=True)


def check_editor(shell, binary, image):
    """Arrow, box and text with a real pointer and keyboard, then Ctrl+S."""
    work = Path(SESSION["KADR_E2E_DIR"]) / f"check-{shell}.png"
    shutil.copy(ROOT / "tests/golden/cases/arrow/base.png", work)
    before = work.read_bytes()
    proc, ms = open_editor(shell, binary, str(work))
    if ms is None:
        return {"opened": False}
    time.sleep(0.5)
    tools = tool_positions(shell, binary)
    pointer("click", *tools["arrow"])
    pointer("drag", 700, 400, 900, 520)
    pointer("click", *tools["rect"])
    pointer("drag", 1000, 450, 1200, 600)
    pointer("click", *tools["text"])
    pointer("click", 760, 650)
    keys("Hello")
    keys("-k", "Escape")
    time.sleep(0.3)
    keys("-M", "ctrl", "s", "-m", "ctrl")
    saved = wait_for(lambda: work.read_bytes() != before, timeout=5)
    stop_all(binary.name)
    # Kept to look at: target/spike-<shell>-check.png.
    shutil.copy(work, ROOT / f"target/spike-{shell}-check.png")
    return {"opened": True, "saved_changed": bool(saved)}


def tool_positions(shell, binary):
    """Where each tool's button is: by the dev hooks in the GPUI app, by the
    spikes' fixed toolbar (left to right from 8, 8) otherwise."""
    if shell == "gpui":
        def middle(id_):
            out = subprocess.run([str(binary), "dev", "find", id_], env=env(), capture_output=True, text=True).stdout
            b = json.loads(out)["result"]["bounds"]
            return (int(b[0] + b[2] / 2), int(b[1] + b[3] / 2))
        return {"arrow": middle("tool-a"), "rect": middle("tool-r"), "text": middle("tool-t")}
    return {"arrow": (40, 24), "rect": (105, 24), "text": (163, 24)}


def measure_build(shell):
    target = ROOT / SHELLS[shell]["touch"]
    original = target.read_bytes()
    subprocess.run([CARGO, "build", "--release", "-p", SHELLS[shell]["package"]], cwd=ROOT, check=True, capture_output=True)
    try:
        target.write_bytes(original + b"\nfn _bench_edit() -> u32 { 7 }\n")
        t0 = time.monotonic()
        subprocess.run([CARGO, "build", "--release", "-p", SHELLS[shell]["package"]], cwd=ROOT, check=True, capture_output=True)
        return time.monotonic() - t0
    finally:
        target.write_bytes(original)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("shell", choices=SHELLS)
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    shell = SHELLS[a.shell]
    binary = ROOT / "target/dist" / shell["bin"]
    if not a.no_build:
        subprocess.run([CARGO, "build", "--profile", "dist", "-p", shell["package"]], cwd=ROOT, check=True)
        subprocess.run([CARGO, "build", "--release", "-p", "kadr-testkit"], cwd=ROOT, check=True, capture_output=True)
    result = {}
    start_session()
    try:
        image = str(Path(SESSION["KADR_E2E_DIR"]) / "4k.png")
        subprocess.run(["magick", "-size", "3840x2160", "plasma:fractal", "-seed", "7", image], check=True)
        times = []
        for _ in range(a.runs):
            proc, ms = open_editor(a.shell, binary, image)
            if ms is not None:
                times.append(ms)
            stop_all(binary.name)
        result["edit_window_ms"] = round(statistics.median(times), 1) if times else None
        proc, _ = open_editor(a.shell, binary, image)
        time.sleep(2)
        pid = proc.pid
        result["editor_4k_rss_mb"] = round(rss_mb(pid), 1)
        result["processes"] = len(tree(pid))
        samples = []
        for _ in range(3):
            before, t0 = cpu_ticks(pid), time.monotonic()
            time.sleep(10)
            samples.append((cpu_ticks(pid) - before) / TICK / (time.monotonic() - t0) * 100)
        result["idle_cpu_percent"] = round(statistics.median(samples), 2)
        stop_all(binary.name)
        result["checks"] = check_editor(a.shell, binary, image)
    finally:
        stop_session()
    result["binary_mb"] = round(binary.stat().st_size / 1024 / 1024, 1)
    if not a.no_build:
        result["incremental_build_s"] = round(measure_build(a.shell), 1)
    out = {"shell": a.shell, "when": time.strftime("%Y-%m-%d %H:%M"), "metrics": result}
    (ROOT / "target").mkdir(exist_ok=True)
    (ROOT / f"target/spike-{a.shell}.json").write_text(json.dumps(out, indent=2) + "\n")
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    sys.exit(main())
