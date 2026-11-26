# Alhambra II: a running light with the debugger

A running light on the [Alhambra II](https://alhambrabits.com/alhambra/)'s
eight LEDs, built with soft-agent's in-design debugger. The debugger lets you
watch the design's signals live and drive them from a page in your browser,
while the design runs on the board. One command does everything, with no AI
agent: it builds the tools, builds the design with the debugger, programs the
board and opens the page.

## What you need

- A Mac with Apple Silicon (the only platform for now).
- An Alhambra II board and its USB cable.
- About 5 GB of disk: the FPGA toolchain (3 GB, in `~/.logix/soft-agent`)
  and the Rust build (1 GB, in the repo's `target` folder).

Get the repo: `git clone https://github.com/zapta/soft-agent` (if macOS
offers to install the command line tools for git, accept, then run the
command again), or download it as a ZIP from GitHub's Code button. Then,
once, from the repo's folder:

```sh
./install-deps.sh
```

It installs the rest: Apple's command line tools (git, Python 3), Homebrew
(it asks first), Rust through rustup, and soft-agent with its FPGA
toolchain. It takes a while the first time, mostly downloading; run it again
whenever a step stopped, as it skips what's already there.

## Run it

Connect the board, then, from the repo's folder:

```sh
examples/alhambra1/run.sh
```

It goes through these steps, saying which one it's on:

1. **Builds soft-agent**, this repo's program, when it changed: a few
   seconds.
2. **Checks the FPGA toolchain** (yosys, nextpnr and the rest), and
   downloads it if `install-deps.sh` hasn't.
3. **Builds the design with the debugger**, into `_build_debug` here, and
   prints how much of the FPGA it uses.
4. **Programs the board.** The LEDs start running.
5. **Opens the debug panel** in your browser. It stays open until you press
   Enter (or Ctrl-C) in the terminal, which releases everything back to the
   design.

Options: `--no-upload` only builds; `--no-open` prints the panel's address
instead of opening it; `--port PORT` picks the serial port when more than one
board is connected.

## Things to try in the panel

- **Watch** shows the design live: `pattern` is the lit LED, `timer` counts
  to 3,000,000 four times a second, and `step_enabled` shows
  what the design sees of `enable_step`: 1, unless you drive it to 0.
- **Hold the light:** click **0** on the `enable_step` row. The light stops
  where it is, and `step_enabled` in Watch shows 0; click **1**, or
  **Release**, to let it run.
- **Step it by hand:** while it's held, click **Pulse** on the
  `enable_step` row. `enable_step` is 1 for a single clock, and the light
  moves one place.
- **Take over the LEDs:** type a pattern on the `leds` row, such as
  `10101010` (it's in binary: one digit per LED, LED7 first), and click
  **Drive**. The LEDs show your pattern. Click **Release** to give them back.
  When released, the light carries on from your pattern, held or not:
  driving `pattern` also feeds the register that holds it.
- Each row's **Format** menu switches between hex, decimal and binary.
- **Plot**, under Watch, draws the watched values over the last seconds or
  minutes: set the refresh rate to 20 or 50 per second for smoother lines,
  and point at the plot to see a moment's values. It only sees what each
  read sees, so a pulse, one clock long, doesn't show.
- **Pause** stops the live updates and freezes the plot; **Read now** reads
  once.

## The files

| File | What it is |
| --- | --- |
| `main.v` | The design: the running light, and a `ticker` module that makes the steps. |
| `pinout.pcf` | Which FPGA pins the clock and the LEDs are on. |
| `debug.toml` | What the debugger watches and drives, and how it reaches the computer. |
| `logix.json` | The board and the top module, for the script. |
| `run.sh` | The one command above. |

Change `debug.toml` to watch or drive other nets, and run `run.sh` again: the
debugger is rebuilt to match. Nets are named by their path from the top
module, such as `main.u_ticker.count`. The debugger is described in
[docs/debugger.md](../../docs/debugger.md).

## If something goes wrong

- **"No board found"**: check the USB cable (some carry power only) and that
  the board's power LED is on.
- **"Can't open /dev/cu.usbserial-…"**: another program has the board's
  serial port, such as a serial monitor, or an earlier `run.sh` still waiting
  for Enter. Close it and try again.
- **The panel says "soft-agent stopped"**: the terminal's `run.sh` ended.
  Run it again.
- **The panel says the debugger didn't answer**: the board was unplugged or
  reprogrammed with another design. Plug it back in: the panel resumes by
  itself; or run `run.sh` again.
