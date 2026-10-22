#!/usr/bin/env python3
"""The budgets of docs/SPEC.ru.md, section 2, measured on Linux (Hyprland).

Writes target/bench.json, prints a table, and fails when a budget is broken;
warns when a number is more than 15% worse than the base in docs/perf.json.

It drives the real session: the area picker shows on the focused monitor for
a moment (and is closed with Escape), an editor window opens and closes. Run
it while not using the machine. Needs grim, wtype, magick, and kadr built
with `cargo build --release` (it is installed to ~/.local/bin first).

  scripts/bench.py [--runs N] [--idle SECONDS] [--no-build] [--save-base]
"""

import argparse
import json
import os
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/release/kadr"
INSTALLED = Path.home() / ".local/bin/kadr"
RUNTIME = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp"))
SOCK = RUNTIME / "kadr.sock"
HYPR = RUNTIME / "hypr" / os.environ.get("HYPRLAND_INSTANCE_SIGNATURE", "") / ".socket.sock"
TICK = os.sysconf("SC_CLK_TCK")
CARGO = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")

# Metric: (budget, unit, lower is better). From the spec's section 2.
BUDGETS = {
    "tray_rss_mb": (15, "MB"),
    "editor_4k_rss_mb": (250, "MB"),
    "idle_cpu_percent": (0.5, "%"),
    "hotkey_overlay_warm_ms": (80, "ms"),
    "hotkey_overlay_cold_ms": (300, "ms"),
    "edit_window_ms": (400, "ms"),
    "binary_mb": (25, "MB"),
    "incremental_build_s": (20, "s"),
}
# Measured, but without a budget of their own.
INFO = {"incremental_fast_s": "s"}


def hypr(request):
    with socket.socket(socket.AF_UNIX) as s:
        s.connect(str(HYPR))
        s.sendall(request.encode())
        out = b""
        while chunk := s.recv(65536):
            out += chunk
    return json.loads(out)


def dev(*args):
    """Ask the running kadr (started with KADR_DEV=1) directly over its socket."""
    try:
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(2)
            s.connect(str(SOCK))
            s.sendall((json.dumps(["dev", *args]) + "\n").encode())
            line = s.makefile().readline()
        return json.loads(line) if line else None
    except OSError:
        return None


def kadr_pids(main_only=True):
    """kadr processes; the main ones (not `kadr tray`) by default."""
    pids = []
    for p in Path("/proc").iterdir():
        if not p.name.isdigit():
            continue
        try:
            argv = (p / "cmdline").read_bytes().split(b"\0")
        except OSError:
            continue
        if not argv or Path(argv[0].decode(errors="replace")).name != "kadr":
            continue
        if main_only and len(argv) > 1 and argv[1] == b"tray":
            continue
        pids.append(int(p.name))
    return pids


def rss_mb(pid):
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1]) / 1024
    return None


def cpu_ticks(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])


def stop_all():
    subprocess.run(["pkill", "-x", "kadr"], check=False)
    for _ in range(200):
        if not kadr_pids(main_only=False):
            break
        time.sleep(0.01)
    try:
        SOCK.unlink()
    except FileNotFoundError:
        pass


def start(*args, dev_hooks=True):
    env = dict(os.environ)
    if dev_hooks:
        env["KADR_DEV"] = "1"
    return subprocess.Popen([str(INSTALLED), *args], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)


def wait_for(check, timeout=5.0, step=0.002):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        v = check()
        if v:
            return v
        time.sleep(step)
    return None


def picker_up(pid):
    """The area picker's layer is mapped for this kadr."""
    for mon in hypr("j/layers").values():
        for level in mon.get("levels", {}).values():
            for layer in level:
                if layer.get("namespace") == "kadr-picker" and layer.get("pid") == pid:
                    return True
    return False


def any_picker():
    for mon in hypr("j/layers").values():
        for level in mon.get("levels", {}).values():
            for layer in level:
                if layer.get("namespace") == "kadr-picker":
                    return layer.get("pid")
    return None


def close_picker():
    subprocess.run(["wtype", "-k", "Escape"], check=False)
    wait_for(lambda: any_picker() is None, timeout=3)


def editor_ready():
    v = dev("state")
    return bool(v and v.get("ok") and (v["result"].get("view") or {}).get("image"))


def close_windows():
    """Close kadr's windows the way Hyprland would."""
    for c in hypr("j/clients"):
        if c.get("pid") in kadr_pids():
            subprocess.run(["hyprctl", "eval", f'hl.dispatch(hl.dsp.window.close({{ window = "address:{c["address"]}" }}))'], capture_output=True, check=False)


def measure_cold_overlay():
    stop_all()
    t0 = time.monotonic()
    proc = start("shot", "region")
    ok = wait_for(lambda: picker_up(proc.pid))
    ms = (time.monotonic() - t0) * 1000 if ok else None
    close_picker()
    stop_all()
    return ms


def measure_warm_overlay(image):
    """A kadr already running (an editor open): the hotkey's `kadr shot region`
    hands over to it."""
    stop_all()
    main = start("edit", image)
    wait_for(editor_ready, timeout=10)
    t0 = time.monotonic()
    subprocess.Popen([str(INSTALLED), "shot", "region"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ok = wait_for(lambda: picker_up(main.pid))
    ms = (time.monotonic() - t0) * 1000 if ok else None
    close_picker()
    stop_all()
    return ms


def measure_edit(image):
    stop_all()
    t0 = time.monotonic()
    start("edit", image)
    ok = wait_for(editor_ready, timeout=10, step=0.005)
    ms = (time.monotonic() - t0) * 1000 if ok else None
    stop_all()
    return ms


def measure_editor_rss_and_cpu(image, seconds=30):
    stop_all()
    proc = start("edit", image)
    wait_for(editor_ready, timeout=10)
    time.sleep(2)
    rss = rss_mb(proc.pid)
    before, t0 = cpu_ticks(proc.pid), time.monotonic()
    time.sleep(seconds)
    cpu = (cpu_ticks(proc.pid) - before) / TICK / (time.monotonic() - t0) * 100
    stop_all()
    return rss, cpu


def measure_tray(idle):
    stop_all()
    proc = start("tray", dev_hooks=False)
    time.sleep(idle)
    rss = rss_mb(proc.pid) if proc.poll() is None else None
    stop_all()
    return rss


def measure_build(profile):
    """One small real change in a UI file (touching it isn't enough: rustc
    sees nothing changed), built again, then put back."""
    target = ROOT / "src/ui.rs"
    original = target.read_bytes()
    try:
        target.write_bytes(original + b"\nfn _bench_edit() -> u32 { 7 }\n")
        t0 = time.monotonic()
        subprocess.run([CARGO, "build", "--profile", profile], cwd=ROOT, check=True, capture_output=True)
        return time.monotonic() - t0
    finally:
        target.write_bytes(original)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--idle", type=float, default=300, help="seconds of tray idle before its RSS is read")
    ap.add_argument("--no-build", action="store_true", help="skip the incremental build timing")
    ap.add_argument("--save-base", action="store_true", help="also write the result as docs/perf.json")
    a = ap.parse_args()

    if not a.no_build:
        subprocess.run([CARGO, "build", "--release"], cwd=ROOT, check=True)
    stop_all()
    shutil.copy2(BIN, INSTALLED)

    tmp = Path(tempfile.mkdtemp(prefix="kadr-bench-"))
    image = str(tmp / "4k.png")
    # A 4K screenshot-like picture (noise, so it doesn't compress to nothing).
    subprocess.run(["magick", "-size", "3840x2160", "plasma:fractal", "-seed", "7", image], check=True)

    def median(f, n):
        values = [v for v in (f() for _ in range(n)) if v is not None]
        return round(statistics.median(values), 1) if values else None

    result = {}
    result["binary_mb"] = round(BIN.stat().st_size / 1024 / 1024, 1)
    result["hotkey_overlay_cold_ms"] = median(measure_cold_overlay, a.runs)
    result["hotkey_overlay_warm_ms"] = median(lambda: measure_warm_overlay(image), a.runs)
    result["edit_window_ms"] = median(lambda: measure_edit(image), a.runs)
    rss, cpu = measure_editor_rss_and_cpu(image)
    result["editor_4k_rss_mb"], result["idle_cpu_percent"] = round(rss, 1), round(cpu, 2)
    result["tray_rss_mb"] = round(measure_tray(a.idle), 1)
    if not a.no_build:
        result["incremental_build_s"] = round(measure_build("release"), 1)
        result["incremental_fast_s"] = round(measure_build("fast"), 1)

    # Leave the tray as it was before.
    subprocess.Popen([str(INSTALLED), "tray"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)

    out = {"when": time.strftime("%Y-%m-%d %H:%M"), "commit": subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip(), "tray_idle_s": a.idle, "runs": a.runs, "metrics": result}
    (ROOT / "target").mkdir(exist_ok=True)
    (ROOT / "target/bench.json").write_text(json.dumps(out, indent=2) + "\n")
    base_path = ROOT / "docs/perf.json"
    base = json.loads(base_path.read_text())["metrics"] if base_path.exists() else {}
    if a.save_base:
        base_path.write_text(json.dumps(out, indent=2) + "\n")

    failed = False
    print(f"{'metric':<26}{'value':>10}{'budget':>10}{'base':>10}")
    for key, (budget, unit) in BUDGETS.items():
        v = result.get(key)
        b = base.get(key)
        mark = ""
        if v is None:
            mark = "  not measured"
        elif v > budget:
            mark, failed = "  OVER BUDGET", True
        elif b and v > b * 1.15:
            mark = "  >15% worse than base"
        print(f"{key:<26}{'-' if v is None else v:>10}{budget:>9}{unit}{'-' if b is None else b:>10}{mark}")
    for key, unit in INFO.items():
        if key in result:
            print(f"{key:<26}{result[key]:>10}{'-':>9}{unit}{base.get(key, '-'):>10}")
    print(f"\nwritten: {ROOT / 'target/bench.json'}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
