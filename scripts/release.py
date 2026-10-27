#!/usr/bin/env python3
"""What the release job publishes beside the files (SPEC 12.2, 12.4).

  scripts/release.py notes VERSION          the release's body: its section of
                                            CHANGELOG.md, then .github/release-notes.md
  scripts/release.py latest VERSION DIR [PREVIOUS]
                                            latest.json for the files in DIR (each
                                            with its .minisig when signed), the
                                            other channel kept from PREVIOUS
"""

import datetime
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = "https://github.com/evlaso/kadr"
# Each platform's file in latest.json, by how its name ends.
PLATFORMS = {
    "linux-x86_64": "-x86_64-linux.tar.xz",
    "macos-universal": "-macos-universal.dmg",
    "windows-x86_64": "-x86_64-windows-setup.exe",
    "windows-x86_64-portable": "-x86_64-windows-portable.exe",
}


def section(changelog, version):
    """The version's section of a Keep a Changelog file, without its heading."""
    m = re.search(rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## |^\[[^\]]+\]: |\Z)", changelog, re.S | re.M)
    if not m:
        sys.exit(f"CHANGELOG.md has no section for {version}")
    return m.group(1).strip()


def notes(version):
    body = section((ROOT / "CHANGELOG.md").read_text(encoding="utf-8"), version)
    extra = (ROOT / ".github/release-notes.md").read_text(encoding="utf-8").replace("{version}", version)
    return f"{body}\n\n{extra}"


def release(version, folder):
    platforms = {}
    for name, ending in PLATFORMS.items():
        f = next(Path(folder).glob(f"*{ending}"), None)
        if f is None:
            sys.exit(f"no *{ending} in {folder}")
        sig = f.with_name(f.name + ".minisig")
        platforms[name] = {
            "url": f"{REPO}/releases/download/v{version}/{f.name}",
            "sha256": hashlib.sha256(f.read_bytes()).hexdigest(),
            "size": f.stat().st_size,
            "sig": sig.read_text(encoding="utf-8") if sig.exists() else "",
        }
    return {"version": version, "date": datetime.date.today().isoformat(), "notes_url": f"{REPO}/releases/tag/v{version}", "platforms": platforms}


def channels(previous):
    """The channels in an earlier latest.json (one written before channels
    is Beta's, and Stable's too when it isn't a pre-release)."""
    if not previous:
        return {}
    if "channels" in previous:
        return dict(previous["channels"])
    old = {k: previous[k] for k in ("version", "date", "notes_url", "platforms") if k in previous}
    return {"beta": old} if "-" in old.get("version", "-") else {"beta": old, "stable": old}


def latest(version, folder, previous=None):
    """Beta always takes the new release; Stable only one without a
    pre-release part. The top level is Beta's, the newest of all, for the
    trays that came before channels."""
    new = release(version, folder)
    chans = channels(previous)
    chans["beta"] = new
    if "-" not in version:
        chans["stable"] = new
    return json.dumps({**new, "channels": chans}, indent=2)


if __name__ == "__main__":
    if sys.argv[1:2] == ["notes"] and len(sys.argv) == 3:
        print(notes(sys.argv[2]))
    elif sys.argv[1:2] == ["latest"] and len(sys.argv) in (4, 5):
        previous = None
        if len(sys.argv) == 5 and Path(sys.argv[4]).exists():
            previous = json.loads(Path(sys.argv[4]).read_text(encoding="utf-8"))
        print(latest(sys.argv[2], sys.argv[3], previous))
    else:
        sys.exit(__doc__)
