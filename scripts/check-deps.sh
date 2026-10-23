#!/usr/bin/env bash
# The dependency rule of docs/SPEC.ru.md section 3: core ← raster ← ui, core ←
# platform; nothing below the app knows a UI framework. Fails with what broke it.
set -euo pipefail
cd "$(dirname "$0")/.."
CARGO=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}

# What each crate may use of kadr's own.
declare -A allowed=(
  [kadr-core]=""
  [kadr-raster]="kadr-core"
  [kadr-platform]="kadr-core"
  [kadr-ui]="kadr-core kadr-raster"
  [kadr-media]="kadr-core kadr-raster"
  [kadr-tray]="kadr-core kadr-platform"
  [kadr-overlay]="kadr-core kadr-raster"
)
frameworks="gpui gpui-ce gpui_ce_platform iced tauri egui slint"

failed=0
for crate in "${!allowed[@]}"; do
  deps=$("$CARGO" tree -p "$crate" -e normal --prefix none --format '{p}' 2>/dev/null | awk '{print $1}' | sort -u)
  for dep in $deps; do
    [ "$dep" = "$crate" ] && continue
    if [[ $dep == kadr* ]] && [[ " ${allowed[$crate]} " != *" $dep "* ]]; then
      echo "$crate must not depend on $dep"; failed=1
    fi
    for f in $frameworks; do
      if [ "$dep" = "$f" ]; then echo "$crate pulls in the UI framework $dep"; failed=1; fi
    done
  done
done
[ $failed = 0 ] && echo "dependencies: ok"
exit $failed
