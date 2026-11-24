#!/bin/bash
# Builds soft-agent if needed, then starts an interactive Claude Code session
# with it connected as the only MCP server (named soft-agent), so you can
# prompt it:
#
#   scripts/run-in-claude.sh            # then type prompts
#   scripts/run-in-claude.sh "What version is the engine?"
#
# Arguments are passed to claude. Claude runs in a playground folder,
# ~/.logix/soft-agent/playground (or SOFT_AGENT_PLAYGROUND), where it can
# create designs, not in this repo. soft-agent's tools are allowed without
# asking.
#
# Like Logix Engine's scripts/run-in-claude.sh, the session starts from a
# clean slate: none of your user settings, no CLAUDE.md files (yours
# included), no auto memory, no IDE connection, and the rules below. Set
# SOFT_AGENT_FULL_SESSION=1 to keep your usual settings and CLAUDE.md files.
#
# Other variables: SOFT_AGENT_HOME, SOFT_AGENT_DATA_FOLDER,
# SOFT_AGENT_LOG_LEVEL (scripts/_rust-env.sh). Logs are in
# <data folder>/logs.

set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/_rust-env.sh

playground="${SOFT_AGENT_PLAYGROUND:-$SOFT_AGENT_HOME/playground}"

claude_bin="$(command -v claude || true)"
if [[ -z "$claude_bin" ]]; then
  claude_bin="$(ls -d "$HOME"/.vscode/extensions/anthropic.claude-code-*/resources/native-binary/claude \
    2> /dev/null | sort -V | tail -n 1 || true)"
fi
if [[ -z "$claude_bin" || ! -x "$claude_bin" ]]; then
  echo "Error: claude not found. Install Claude Code: https://claude.com/claude-code" >&2
  exit 1
fi

echo "Building soft-agent..." >&2
server="$(build_and_copy)"
echo "$("$server" --version), engine folder $engine_folder, log level $log_level" >&2
echo "Claude Code: $claude_bin, in $playground" >&2

mkdir -p "$playground"
cd "$playground"

mcp_config=$(cat <<JSON
{"mcpServers": {"soft-agent": {"command": "$server", "args": ["--engine-folder", "$engine_folder", "--log-level", "$log_level"]}}}
JSON
)

session_rules="This is a test session of soft-agent, Logix Engine with experimental extension \
tools. Start only from what the soft-agent MCP server provides (its instructions, its tool \
descriptions and the docs they link to) and from the folders the user names. Don't read \
source repos or other local clones, other projects' files, or notes and memories from \
earlier sessions. If something you need is missing, say so instead of looking for it elsewhere."

if [[ "${SOFT_AGENT_FULL_SESSION:-}" == 1 ]]; then
  exec "$claude_bin" \
    --strict-mcp-config \
    --mcp-config "$mcp_config" \
    --allowedTools "mcp__soft-agent" \
    "$@"
fi

claude_settings=$(cat <<JSON
{"claudeMdExcludes": ["$HOME/.claude/CLAUDE.md", "**/CLAUDE.md", "**/CLAUDE.local.md"]}
JSON
)
CLAUDE_CODE_DISABLE_AUTO_MEMORY=1 CLAUDE_CODE_AUTO_CONNECT_IDE=false exec "$claude_bin" \
  --setting-sources project,local \
  --settings "$claude_settings" \
  --append-system-prompt "$session_rules" \
  --strict-mcp-config \
  --mcp-config "$mcp_config" \
  --allowedTools "mcp__soft-agent" \
  "$@"
