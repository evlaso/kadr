#!/usr/bin/env bash
# Uses the existing release binary; all app runs stay in nested Hyprland.
set -euo pipefail
exec python3 "$(dirname "$0")/shots.py" "$@"
