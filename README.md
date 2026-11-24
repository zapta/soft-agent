# soft-agent

Experimental extensions to [Logix Engine](https://github.com/openlogix/logix-engine).
soft-agent is an MCP server that has all of the engine's tools, plus tools of
its own. Ideas that work out may later move into the engine itself.

It uses the engine as a Rust library (the `logix-engine` crate, pinned to a
commit in `Cargo.toml`), not as a separate process:

- Engine tools are passed to the engine's own MCP server code unchanged, so
  they behave exactly as in `logix-engine`.
- Extension tools live in [src/tools.rs](src/tools.rs) and are listed after
  the engine's. They use the engine's `EngineTool` trait, so their schemas,
  input checks and errors look the same. For now there is one,
  `get_soft_agent_info`.
- The command line is the engine's (`--engine-folder`, `--log-level`, ...,
  see the engine's `docs/command-line.md`).

## Requirements

- macOS on Apple Silicon (as the engine, for now).
- Rust via rustup (`brew install rustup`). The first build installs the
  version pinned in `rust-toolchain.toml`.
- [Claude Code](https://claude.com/claude-code), signed in.

## Run Claude Code against it

```sh
scripts/run-in-claude.sh
```

This builds soft-agent, then starts Claude Code in a playground folder
(`~/.logix/soft-agent/playground`) with soft-agent as its only MCP server.
Try prompts such as "What extension tools do you have?" or "Make an LED blink
on the iCESugar v1.5 board". The first engine call downloads the FPGA tool
packages into `~/.logix/soft-agent/engine-data` (a few minutes, once).

By default the session starts from a clean slate (no user settings,
CLAUDE.md files or memories), as the engine's own test script does. Set
`SOFT_AGENT_FULL_SESSION=1` to keep them. See the script's header for the
other variables.

To have soft-agent in every Claude Code session instead:

```sh
claude mcp add --scope user soft-agent -- "$PWD/scripts/serve.sh"
```

## Engine version

`Cargo.toml` pins `logix-engine` to a commit (`rev`). To move to a newer
engine:

1. Change `rev`, then `cargo update -p logix-engine` and build.
2. Compare the engine's `src/main.rs` with ours: our startup copies it,
   since it's in the engine's binary, not its library.
3. Keep `rust-toolchain.toml` at the engine's Rust version.

To build against a local engine clone instead (to try engine changes
together with extension changes), create `.cargo/config.toml` (git-ignored):

```toml
[patch."https://github.com/openlogix/logix-engine"]
logix-engine = { path = "/absolute/path/to/logix-engine" }
```

## Development

```sh
cargo build --release
cargo clippy --release
```
