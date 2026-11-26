#!/usr/bin/env bash
# Installs what soft-agent and its examples need, on a Mac with Apple
# Silicon:
#
#   ./install-deps.sh
#
# 1. Apple's command line tools (git, Python 3, the C linker Rust needs).
# 2. Homebrew, after asking.
# 3. rustup, through Homebrew.
# 4. The Rust version this repo builds with (rust-toolchain.toml).
# 5. soft-agent itself, built once.
# 6. The FPGA toolchain soft-agent uses, about 3 GB, in ~/.logix/soft-agent.
#
# Each step checks first and skips what's already there, so it's safe to run
# again, such as after a step stopped.

set -euo pipefail
cd "$(dirname "$0")"

step() { printf '\n== %s\n' "$1"; }
fail() { printf '\nError: %s\n' "$1" >&2; exit 1; }

step "Checking the computer"
[[ "$(uname -s)" == "Darwin" ]] || fail "this script is for macOS; soft-agent runs on Macs with Apple Silicon for now."
[[ "$(uname -m)" == "arm64" ]] || fail "this Mac has an Intel processor; soft-agent runs on Macs with Apple Silicon for now."
echo "macOS on Apple Silicon."

step "Apple's command line tools"
if xcode-select -p > /dev/null 2>&1; then
  echo "Installed."
else
  xcode-select --install > /dev/null 2>&1 || true
  echo "macOS is asking to install them in a window of its own: accept it,"
  echo "wait until it finishes, then run ./install-deps.sh again."
  exit 1
fi

step "Homebrew"
brew=""
for b in /opt/homebrew/bin/brew "$(command -v brew 2> /dev/null || true)"; do
  if [[ -n "$b" && -x "$b" ]]; then brew="$b"; break; fi
done
if [[ -n "$brew" ]]; then
  echo "Installed."
else
  echo "Homebrew (https://brew.sh) isn't installed. Its official installer will"
  echo "ask for your password, then install it in /opt/homebrew."
  read -r -p "Install Homebrew now? [y/N] " answer
  [[ "$answer" =~ ^[Yy] ]] || fail "Homebrew is needed for rustup: install it from https://brew.sh, then run this again."
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  brew=/opt/homebrew/bin/brew
  [[ -x "$brew" ]] || fail "Homebrew didn't install. See https://brew.sh, then run this again."
fi

step "rustup"
rustup_bin="$("$brew" --prefix)/opt/rustup/bin"
if [[ -x "$rustup_bin/rustup" ]]; then
  echo "Installed."
elif command -v rustup > /dev/null 2>&1; then
  rustup_bin="$(dirname "$(command -v rustup)")"
  echo "Installed (not from Homebrew: $rustup_bin)."
else
  "$brew" install rustup
fi
export PATH="$rustup_bin:$PATH"

step "Rust $(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
# Reads rust-toolchain.toml; quick when it's already installed.
rustup toolchain install
cargo --version

step "Building soft-agent (the first time takes a few minutes)"
source scripts/_rust-env.sh
server="$(build_and_copy)"
echo "Built: $server"

step "The FPGA toolchain (about 3 GB, the first time)"
python3 -I scripts/download_toolchain.py --server "$server" --engine-folder "$engine_folder"

step "Done"
echo "Connect an Alhambra II and run: examples/alhambra1/run.sh"
