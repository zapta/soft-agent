# Alhambra II: a running light with the debugger

A running light on the [Alhambra II](https://alhambrabits.com/alhambra/)'s
eight LEDs, built with soft-agent's in-design debugger. The debugger lets you
watch the design's signals live and drive them from a page in your browser,
while the design runs on the board. One command does everything, with no AI
agent: it builds the tools, builds the design with the debugger, programs the
board and opens the page.

## Get started

**New to it?** Follow [TUTORIAL.md](TUTORIAL.md). It starts from a clone of
this repo and an Alhambra II on a Mac with Apple Silicon, and goes through
installing the tools, building, programming the board and using the debug
panel, step by step.

**In short:** once, from the repo's folder, install the tools (Apple's
command line tools, Homebrew, Rust, soft-agent and its FPGA toolchain,
about 5 GB in all):

```bash
./install-deps.sh
```

Then, with the board connected, build the design with the debugger, program
the board and open the panel:

```bash
examples/alhambra1/run.sh
```

Press Return in the terminal to close the panel and release everything back
to the design. Options: `--no-upload` only builds; `--no-open` prints the
panel's address instead of opening it; `--port PORT` picks the serial port
when more than one board is connected.

## The files

| File | What it is |
| --- | --- |
| `main.v` | The design: the running light, and a `ticker` module that makes the steps. |
| `pinout.pcf` | Which FPGA pins the clock and the LEDs are on. |
| `debug.toml` | What the debugger watches and drives, and how it reaches the computer. |
| `logix.json` | The board and the top module, for the script. |
| `run.sh` | The one command above. |
| `TUTORIAL.md` | The step-by-step tutorial. |

Change `debug.toml` to watch or drive other nets, and run `run.sh` again: the
debugger is rebuilt to match. Nets are named by their path from the top
module, such as `main.u_ticker.count`. The debugger is described in
[docs/debugger.md](../../docs/debugger.md).

If something goes wrong, see the tutorial's
[If something goes wrong](TUTORIAL.md#if-something-goes-wrong).
