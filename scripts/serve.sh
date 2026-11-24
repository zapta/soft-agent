#!/bin/bash
# Builds soft-agent if needed and runs it as an MCP server on stdin/stdout.
# For registering it with Claude Code (or another MCP host) so every session
# gets the latest build:
#
#   claude mcp add --scope user soft-agent -- <this repo>/scripts/serve.sh
#
# Undo with `claude mcp remove --scope user soft-agent`. Build output goes
# to stderr, so it doesn't disturb MCP on stdout.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

server="$(build_and_copy)"
exec "$server" --engine-folder "$engine_folder" --log-level "$log_level" "$@"
