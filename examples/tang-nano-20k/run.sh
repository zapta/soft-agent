#!/usr/bin/env bash
# Builds this example with the debugger, programs the Tang Nano 20K and opens
# the debug panel. No AI agent involved. See README.md here.

exec "$(dirname "$0")/../../scripts/debug-run.sh" "$(cd "$(dirname "$0")" && pwd)" "$@"
