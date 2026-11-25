#!/usr/bin/env bash
# Opens the debug panel for a debug build on the board, from the shell:
#
#   scripts/debug-panel.sh ~/my-design/_build_debug [--port PORT] [--no-open]
#
# Builds soft-agent if needed, opens a debug session and the panel in the
# browser, and waits: Enter or Ctrl-C closes the session, which releases
# every override and frees the serial port. Without --port it uses the one
# connected board's serial port. See scripts/debug_panel.py.

set -euo pipefail
started_in="$PWD"
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

server="$(build_and_copy)"
exec python3 -I scripts/debug_panel.py --server "$server" --engine-folder "$engine_folder" \
  --cwd "$started_in" "$@"
