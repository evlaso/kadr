#!/usr/bin/env python3
"""What the release job publishes beside the files (SPEC 12.2, 12.4).

  scripts/release.py notes VERSION          the release's body: its section of
                                            CHANGELOG.md, then .github/release-notes.md
  scripts/release.py latest VERSION DIR     latest.json for the files in DIR
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
PLATFORMS = {"linux-x86_64": "-x86_64-linux.tar.xz", "macos-universal": "-macos-universal.dmg", "windows-x86_64": "-x86_64-windows-setup.exe"}


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


def latest(version, folder):
    platforms = {}
    for name, ending in PLATFORMS.items():
        f = next(Path(folder).glob(f"*{ending}"), None)
        if f is None:
            sys.exit(f"no *{ending} in {folder}")
        platforms[name] = {"url": f"{REPO}/releases/download/v{version}/{f.name}", "sha256": hashlib.sha256(f.read_bytes()).hexdigest(), "sig": ""}
    return json.dumps({"version": version, "date": datetime.date.today().isoformat(), "notes_url": f"{REPO}/releases/tag/v{version}", "platforms": platforms}, indent=2)


if __name__ == "__main__":
    if sys.argv[1:2] == ["notes"] and len(sys.argv) == 3:
        print(notes(sys.argv[2]))
    elif sys.argv[1:2] == ["latest"] and len(sys.argv) == 4:
        print(latest(sys.argv[2], sys.argv[3]))
    else:
        sys.exit(__doc__)
