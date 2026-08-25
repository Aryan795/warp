#!/usr/bin/env bash
# Runs the raw-keypress ctrl-r handoff shell-side test matrix (CORE-3807).
# See raw_keypress_ctrl_r_matrix.py for what it covers.
set -uo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
exec python3 "$HERE/raw_keypress_ctrl_r_matrix.py" "$@"
