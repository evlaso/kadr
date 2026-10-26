#!/usr/bin/env bash
# A release's files for Linux (an archive) or Windows (an installer and a
# portable kadr.exe) (SPEC 12.2), from the dist build
# (`cargo build --profile dist -p kadr -p kadr-tray -p kadr-overlay`), with a
# .sha256 beside it. macOS has its own: scripts/bundle-macos.sh.
#
#   scripts/package.sh linux|windows VERSION OUT_DIR
set -euo pipefail

os=$1
version=$2
out=$3
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d)

case $os in
linux)
    name="kadr-$version-x86_64-linux"
    dir="$stage/$name"
    mkdir -p "$dir"
    # The area overlay is a thread of kadr on Linux: no kadr-overlay.
    install -m755 "$root/target/dist/kadr" "$root/target/dist/kadr-tray" "$dir/"
    cp "$root/data/hypr/kadr.lua" "$root/LICENSE" "$root/README.md" "$root/CHANGELOG.md" "$dir/"
    cat >"$dir/INSTALL.md" <<'EOF'
# Installing kadr on Linux (Hyprland)

kadr needs `gpu-screen-recorder`, `ffmpeg`, `tesseract` (with English data), `grim` and `wl-clipboard`.

```sh
install -m755 kadr kadr-tray ~/.local/bin/
cp kadr.lua ~/.config/hypr/kadr.lua
```

Add `require("hypr.kadr")` to `~/.config/hypr/hyprland.lua` (it floats kadr's windows, loads the hotkeys set in kadr's settings and logs clicks and keys for the studio), and start the tray from `autostart.lua`:

```lua
o.launch_on_start("kadr-tray")
```

`kadr --help` lists the commands; `kadr settings` opens the settings.
EOF
    file="$name.tar.xz"
    tar -C "$stage" -cJf "$out/$file" "$name"
    ;;
windows)
    # An installer and a portable kadr.exe, no archive: the portable one is
    # its own tray (`kadr tray`) and area overlay (`kadr overlay`).
    dist=$root/target/dist
    portable="kadr-$version-x86_64-windows-portable.exe"
    cp "$dist/kadr.exe" "$out/$portable"
    iscc=$(command -v iscc || command -v ISCC || echo "/c/Program Files (x86)/Inno Setup 6/ISCC.exe")
    # Git Bash would turn /D… into paths.
    MSYS2_ARG_CONV_EXCL='*' "$iscc" /Q "/DVersion=$version" "/DSource=$(cygpath -w "$dist")" "/DOut=$(cygpath -w "$out")" "$(cygpath -w "$root/packaging/windows/kadr.iss")"
    file="kadr-$version-x86_64-windows-setup.exe"
    (cd "$out" && sha256sum "$portable" >"$portable.sha256")
    ;;
*)
    echo "usage: $0 linux|windows VERSION OUT_DIR" >&2
    exit 2
    ;;
esac

rm -rf "$stage"
(cd "$out" && sha256sum "$file" >"$file.sha256")
echo "$out/$file"
