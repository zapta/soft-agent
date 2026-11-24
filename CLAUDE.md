# CLAUDE.md

soft-agent: experimental extensions to Logix Engine (`openlogix/logix-engine`),
built as an MCP server that uses the engine as a Rust library. README.md has
the overview, how to run it and how to bump the engine.

## Layout

- `src/main.rs`: startup, copied from the engine's `src/main.rs` (binary
  only, not in the engine's library). Keep it in step when bumping the engine,
  including the self-exec dispatch (diagram layout worker, Windows driver
  helper): the engine runs `current_exe`, which is this binary.
- `src/server.rs`: wraps `logix_engine::server::EngineServer`. Engine tools are
  delegated to it unchanged; extension tools are handled first.
- `src/tools.rs`: the extension tools, each a `logix_engine::tools::EngineTool`.
- `scripts/run-in-claude.sh`, `scripts/serve.sh`: build and run for Claude Code.

## Rules

- Don't change engine behavior by patching around it here. Extensions add
  tools (or wrap the server); a change the engine itself needs goes to the
  engine repo, following its own CLAUDE.md.
- Extension tool names must not clash with engine tool names (the engine's
  `tools::all_definitions()`).
- Write extensions so they could move into the engine later: same tool
  conventions as the engine's `docs/tool-api.md` (self-contained requests,
  absolute paths, errors as `ToolError` data).
- Before committing, `cargo build --release` and `cargo clippy --release` are
  clean.
