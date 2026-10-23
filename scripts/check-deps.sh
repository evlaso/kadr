#!/usr/bin/env bash
# The dependency rule of docs/SPEC.ru.md section 3: core ← raster ← ui, core ←
# platform; GPUI only in kadr-ui-gpui, nothing else knows a UI framework.
# Fails with what broke it.
set -euo pipefail
cd "$(dirname "$0")/.."
CARGO=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}

# What each crate may use of kadr's own, through everything it depends on.
declare -A allowed=(
  [kadr-core]=""
  [kadr-raster]="kadr-core"
  [kadr-platform]="kadr-core"
  [kadr-ui]="kadr-core kadr-raster"
  [kadr-media]="kadr-core kadr-raster"
  [kadr-tray]="kadr-core kadr-platform"
  [kadr-overlay]="kadr-core kadr-raster"
)
# The two on top, by what they use themselves: the UI, and the program
# (its command line), which starts the UI but doesn't touch GPUI.
declare -A direct=(
  [kadr-ui-gpui]="kadr-core kadr-raster kadr-ui kadr-platform kadr-media kadr-tray kadr-overlay"
  [kadr]="kadr-core kadr-platform kadr-tray kadr-ui-gpui"
)
frameworks="gpui gpui-ce gpui_ce_platform iced tauri egui slint"

failed=0
check() {
  local crate=$1 allow=$2 depth=$3
  local deps
  deps=$("$CARGO" tree -p "$crate" -e normal --prefix none --format '{p}' $depth 2>/dev/null | awk '{print $1}' | sort -u)
  for dep in $deps; do
    [ "$dep" = "$crate" ] && continue
    if [[ $dep == kadr* ]] && [[ " $allow " != *" $dep "* ]]; then
      echo "$crate must not depend on $dep"; failed=1
    fi
    for f in $frameworks; do
      if [ "$dep" = "$f" ] && [ "$crate" != kadr-ui-gpui ]; then echo "$crate pulls in the UI framework $dep"; failed=1; fi
    done
  done
}
for crate in "${!allowed[@]}"; do check "$crate" "${allowed[$crate]}" ""; done
for crate in "${!direct[@]}"; do check "$crate" "${direct[$crate]}" "--depth 1"; done

# And in the code: GPUI is named nowhere outside kadr-ui-gpui (and the
# vendored GPUI CE crate).
if hits=$(grep -rnE '\bgpui(_platform)?::|\buse gpui|extern crate gpui' --include='*.rs' src crates tests | grep -v '^crates/kadr-ui-gpui/'); then
  echo "GPUI outside kadr-ui-gpui:"; echo "$hits"; failed=1
fi
[ $failed = 0 ] && echo "dependencies: ok"
exit $failed
