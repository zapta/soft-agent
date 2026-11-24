# Sourced by the other scripts. Makes sure cargo is on the PATH: if it isn't,
# uses Homebrew's rustup, which isn't on the PATH by default.

if ! command -v cargo > /dev/null 2>&1; then
  for dir in /opt/homebrew/opt/rustup/bin /usr/local/opt/rustup/bin "$HOME/.cargo/bin"; do
    if [[ -x "$dir/cargo" ]]; then
      export PATH="$dir:$PATH"
      break
    fi
  done
fi

if ! command -v cargo > /dev/null 2>&1; then
  echo "Error: cargo not found. Install Rust with rustup: 'brew install rustup'" \
    "on macOS, or see https://rustup.rs." >&2
  exit 1
fi

# Where the scripts keep things, outside the repo. Each can be overridden.
SOFT_AGENT_HOME="${SOFT_AGENT_HOME:-$HOME/.logix/soft-agent}"
# The engine folder: packages and logs. Its name must contain "engine".
engine_folder="${SOFT_AGENT_DATA_FOLDER:-$SOFT_AGENT_HOME/engine-data}"
# error, warn, info (default) or debug (also logs every program's output).
log_level="${SOFT_AGENT_LOG_LEVEL:-info}"

# Builds the release binary and copies it to $SOFT_AGENT_HOME/engine-program,
# so a running session keeps its copy while you rebuild, and the agent's MCP
# config doesn't show the repo's path. Prints the copy's path.
build_and_copy() {
  cargo build --release --locked >&2
  local bin="$SOFT_AGENT_HOME/engine-program"
  mkdir -p "$bin"
  cp target/release/soft-agent "$bin/.soft-agent.tmp"
  mv -f "$bin/.soft-agent.tmp" "$bin/soft-agent"
  echo "$bin/soft-agent"
}
