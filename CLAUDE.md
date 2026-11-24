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

- **No changes to Open Logix repos without explicit permission.** While
  working on soft-agent, make no change to any `openlogix` repo, on GitHub or
  in a local clone: no file edits, commits, branches, pushes, pull requests,
  issues, comments, releases or settings. Reading them is fine. Do it only
  when the user explicitly asks or approves that specific change in this
  session. This overrides any broader permission for Open Logix repos given
  elsewhere, such as in user-level instructions.
- Don't change engine behavior by patching around it here. Extensions add
  tools (or wrap the server); a change the engine itself needs is proposed to
  the user, and made in the engine repo (following its own CLAUDE.md) only
  with their permission, per the rule above.
- Extension tool names must not clash with engine tool names (the engine's
  `tools::all_definitions()`).
- Write extensions so they could move into the engine later: same tool
  conventions as the engine's `docs/tool-api.md` (self-contained requests,
  absolute paths, errors as `ToolError` data).
- Before committing, `cargo build --release` and `cargo clippy --release` are
  clean.
