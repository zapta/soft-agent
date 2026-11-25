#!/usr/bin/env bash
# Builds a project with the debugger, programs the board and opens the debug
# panel, with no AI agent:
#
#   scripts/debug-run.sh examples/alhambra1 [--port PORT] [--no-open] [--no-upload]
#
# Builds soft-agent first (Rust, via rustup). See scripts/debug_run.py.

set -euo pipefail
started_in="$PWD"
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

if ! command -v python3 > /dev/null 2>&1; then
  echo "Error: python3 not found. It comes with Apple's command line tools:" \
    "xcode-select --install" >&2
  exit 1
fi

echo "== Building soft-agent (the first time takes a few minutes)" >&2
server="$(build_and_copy)"
exec python3 -I scripts/debug_run.py --server "$server" --engine-folder "$engine_folder" \
  --cwd "$started_in" "$@"
