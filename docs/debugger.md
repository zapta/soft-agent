# In-design debugger: requirements and design

This is the spec for soft-agent's in-design debugger. It started as a Claude
Doc, where the options were discussed and decided; this file is now the
source of truth, and the doc is obsolete.

## Summary and goals

A debug build adds a small debugger to the user's design, so a host can read
and write chosen nets while the design runs on the board. A debug
configuration file names the nets; the build inserts the debugger and its link
to the host (a UART over the board's USB cable first). Everything is
deterministic tooling: no AI at build time or run time. AI agents are only one
possible client of the host tools, like any other.

Goals:

- **Observe:** read the current value of named nets (state machines,
  counters, flags, bus registers) without adding ports or editing the design.
- **Control:** write named nets or registers, such as forcing an enable,
  injecting a value or holding a reset.
- **No design changes needed:** a design builds the same with or without
  debugging; the debug build only adds.
- **Movable:** built so the feature can later move into Logix Engine without
  changing the file format or the tools.

**Next phase, planned:** capture. The debugger records chosen signals on the
board and sends them up to the host, to view as a waveform (like ILA or
SignalTap). The first version leaves room for it in the file format, the
debugger and the protocol, as each section notes.

Out of scope for both: on-chip CPUs, JTAG uplinks and multi-board setups.

## Background: how designs build today

The engine's `build` tool runs one yosys call, then the toolchain's place and
route and packer. The debug build has to fit around that flow, since
soft-agent uses the engine unchanged.

- **Synthesis:** one yosys run reads the listed `.v`/`.sv` files with `-D`
  defines (`SYNTHESIZE` always) and runs `hierarchy -top <top>; setattr …;
  synth_ice40 -top <top> -json hardware.json` (`synth_ecp5`, `synth_gowin`,
  `synth_xilinx` for the other families). The script is fixed by
  `build::synth_script`; a request can't add passes to it.
- **Place and route, packing:** `nextpnr-ice40` with the constraint file, then
  `icepack` (iCE40); the other families have their own pair.
- **Inputs are explicit:** every request names its source folder, source
  files, top module, constraint file and build folder. The engine searches for
  nothing and keeps no project file.
- **Board facts come from the catalog:** `get_catalog` gives each board's
  clock (pin, MHz) and UART pins. The iCESugar v1.5 has a 12 MHz clock on pin
  35 and its USB UART on TX 6, RX 4.
- **Serial is already there:** `open_serial`, `read_serial`, `write_serial`
  and `close_serial` keep a session on a board's UART (8N1, 115200 by
  default), pause it while `program_board` writes that board, and reopen it
  after.
- **Extension point:** soft-agent can add tools and call the engine's public
  library code, but can't change the engine's `build`. A debug build therefore
  prepares sources that the engine's build then synthesizes (Build
  integration, below).

## Requirements

The first version is the Must rows; Should rows are wanted if cheap; the Next
phase row (capture) is planned and shapes today's choices; Later rows only
constrain them.

| ID | Requirement | Priority |
| --- | --- | --- |
| R1 | A debug configuration file names the nets to read and to write, by hierarchical name from the top module (such as `main.u_ctrl.state`); widths come from the design. | Must |
| R2 | Nets can also be marked in the design itself (an attribute or a probe module), and the build finds them. | Later |
| R3 | A debug build of an unchanged design adds the debugger; a normal build of the same sources has no trace of it. | Must |
| R4 | The host reads any listed net, and writes any net listed as writable, by name. | Must |
| R5 | Reads of several nets in one request are taken in the same clock cycle (a coherent snapshot). | Must |
| R6 | The uplink is the board's UART from the catalog, at a baud rate the board's clock supports. | Must |
| R7 | The build writes a debug map (names, widths, directions, addresses, clock) into the build folder, and the host tools take names from it, never from the sources. | Must |
| R8 | The debugger reports an ID of the build it is in, and the host refuses a map that doesn't match the bitstream on the board. | Must |
| R9 | A name that doesn't exist or was optimized away is a design error at build time. | Must |
| R10 | The debugger's resource use shows in the build report, apart from the design's. | Should |
| R11 | Nets in other clock domains than the debugger's are read safely (synchronized), and the map says which are. | Later |
| R12 | When the design already uses the UART pins, the build refuses with a clear error, or the configuration picks other pins. | Should |
| R13 | Works for every family the engine builds (iCE40, ECP5, Gowin, Xilinx 7), with no per-family code in the debugger. | Should |
| R14 | Capture (the planned next phase): record listed signals on the board, such as into block RAM from a trigger condition, and send them up to the host as a waveform. The first version's file, debugger and protocol leave room for it. | Next phase |
| R15 | Other uplinks: JTAG, SPI, USB through a soft USB core. | Later |

Non-functional: the debugger stays small (a target under 300 iCE40 logic
cells for 8 probes; to be measured), it must not lower the design's maximum
clock much, and the protocol and file format are documented and versioned, so
other hosts can use them without soft-agent.

## Languages

Verilog and SystemVerilog, the same as the engine. The debugger works on the
netlist yosys builds from the source, so net names are the same instance
paths in either language. It covers what the engine's yosys reader
(`read_verilog -sv`) accepts. Version 1 takes plain nets and registers;
SystemVerilog struct fields, interface signals and packed-array slices need
their own naming rules and come later. No VHDL yet, since the engine doesn't
support it.

## The debug configuration file

One JSONC (JSON with comments) file per debug build, named in the build
request like the constraint file (such as `debug.jsonc`), with a schema
generated from Rust types as the engine does for its tools. It says which nets
the debugger sees, how it reaches the host, and which clock it runs on.

Format options considered:

| Option | For | Against |
| --- | --- | --- |
| JSON with a generated schema | Same as the engine's tools and catalog; editors validate it; easy for any host to write | No comments; verbose for long lists |
| JSONC (JSON with comments), chosen | Comments for people; the engine already reads JSONC remote configs | Fewer editors validate it out of the box |
| TOML | Short and readable for flat lists | Nested probe groups are awkward; a second format in the project |
| A constraint-style text file (like `.pcf`, one line per net) | Very short; familiar to FPGA users | A new grammar to parse and document; hard to extend |

An example, for the iCESugar v1.5:

```jsonc
{
  "$schema": "https://raw.githubusercontent.com/zapta/soft-agent/main/schemas/logix-debug-v1.json",
  "clock": "sys_clk",
  // Everything in uplink can be left out, as "uplink": {}: kind defaults
  // to uart, tx and rx to the board's UART, baud to 115200.
  "uplink": { "kind": "uart", "tx": "6", "rx": "4", "baud": 115200 },
  "input": [
    { "net": "main.count", "name": "counter" },
    { "net": "main.blink", "name": "blink" },
    { "net": "main.u_ctrl.state" }
  ],
  "output": [
    { "net": "main.blink", "name": "blink_force", "mode": "override" }
  ]
}
```

- `$schema`: optional, as in the catalog's board files: editors check and
  complete the file, and the build refuses any other value. The version lives
  in the address (`-v1`; a breaking change gets `-v2`), so there is no
  separate `version` field. The schema lives in this repo, under `schemas/`.
- `clock`: the one clock the debugger and its UART run on, a top-level port of
  the design; every input and output net must be on it. Its frequency comes
  from the board's clock resource when it is that pin, else from
  `clock_mhz` in the file.
- `uplink.tx`, `uplink.rx`: the UART's pins. Each is optional and defaults to
  the board's UART in the catalog (the iCESugar v1.5's TX 6 and RX 4), so most
  files can leave them out; other pins are for a USB serial adapter on a
  PMOD. `baud` defaults to 115200, and `kind` to `uart`, the only kind in
  version 1. `"uplink": {}` takes every default; it says the same as leaving
  `uplink` out, but shows a reader where the link is set.
- `input` and `output`, from the debugger's side, as AMD's VIO names
  probe\_in and probe\_out: an input is a net it reads, an output a net it
  drives. `name` is the host's name for it (defaults to the net's path); widths
  come from the design, not the file.
- `mode` of an output: `override` (the debugger can take over a net the
  design drives, and releases it back to the design) or `register` (a net
  with no driver in the design, driven only by the debugger). See the next
  section.

For the capture phase, the file gains a `capture` section alongside `input`
and `output` (signals, depth, trigger, sample clock). Version 1 reserves the
key and refuses it with "not supported yet", so files written now stay valid
later.

Decided: one debug file per build request; merging several files can come
later.

## Connecting the debugger to design nets

Decided: the first version takes nets from the debug file only (option A).
Marking in the design (B, C) comes later, merged into the same list. Reads
are easy either way; writes are the hard part, since the debugger must take
over a net that the design already drives.

| Option | How it looks | For | Against |
| --- | --- | --- | --- |
| A. Names in the file (version 1) | `"net": "main.u_ctrl.state"` | No design edits at all (R3); one place to see what is debugged | Names break when code is renamed; synthesis renames and removes nets, so the build must keep them alive and say when it can't (R9) |
| B. Attributes in the design (later) | `(* logix_debug = "input" *) reg [3:0] state;` | Travels with the code, survives renames; a normal build ignores unknown attributes | Edits the design; attributes alone don't say the host name or write mode unless their value grows a small syntax |
| C. Probe modules in the design (later) | `logix_probe #(.NAME("state")) p (.value(state));` and `logix_source #(.NAME("en")) s (.design(en_logic), .value(en));` | Explicit and simulatable: a testbench sees the same pass-through; a write is a clean mux the designer placed | A normal build needs the probe modules' pass-through file too; most design edits of the three |

B and C differ in kind: an attribute is a note that adds no hardware, so the
build must find the net and insert any override mux itself; a probe module is
real hardware in the design, so simulation sees it and the designer places the
override mux.

Writes, by mode:

- **register:** the debugger drives a net nothing else drives (a spare
  control input). Simplest: no net is cut.
- **override:** the debugger can take over a net the design drives, through a
  mux at that net's driver, released by default. With option A the build cuts
  the net and inserts the mux itself, a netlist edit (Build integration).

### Naming rules (version 1)

- A name is a full instance path, dot-separated, starting with the top
  module's name, then the net: `main.u_core.u_fifo.count`. A net in the top
  module itself is `main.count`. Starting with the top makes a path read the
  same as in a testbench or a waveform, and the build checks it against
  `top_module`.
- Each instance is its own net: a module instantiated twice gives
  `main.u_lane0.err` and `main.u_lane1.err`, and the file lists each one it wants.
- Generate blocks are written as yosys names them: `main.gen_lane[2].u_lane.err`.
- Whole nets only: no bit selects (`bus[7:4]`) and no wildcards in version 1.
- A name that matches no net, or more than one, is a build error (R9).

Capture phase: the same selection marks capture signals (`"capture"` in the
file), so nothing here changes.

## Debugger hardware

A small register-map debugger with a UART command engine, in plain
Verilog-2005, with no CPU and no family primitives. The host sends short
frames; the debugger answers from registers that tap or drive the design's
nets.

```
            FPGA                                     board            host
 +---------------------------------------------+
 |  user design (instrumented copy)            |
 |    nets to read ---------+                  |
 |    nets to override <-+  |                  |
 |                       |  v                  |
 |  +--------------------------------------+   |
 |  | debug hub                            |   |
 |  |  snapshot regs | write regs + enables|   |
 |  |  build ID ROM  | [capture: later]    |   |
 |  +--------------------------------------+   |
 |            ^ register bus                   |
 |  +--------------------------------------+   |   UART   +--------+  USB  +-----------+
 |  | command engine + UART                |<--+--------->| bridge |<----->| soft-agent|
 |  +--------------------------------------+   |          +--------+       | MCP server|
 +---------------------------------------------+                           +-----------+
```

- **Debug hub:** one snapshot register per read net, all loaded in the same
  cycle on a SNAPSHOT command (R5); one register per write net, plus an
  override enable per override net; a ROM with the build ID and protocol
  version (R8).
- **Command engine + UART:** parses frames and answers them. At 12 MHz and
  115200 baud the divisor is 104 (0.16% off), well within UART tolerance.
- **Clock domains:** version 1 has one clock, shared by the UART and the
  debugger, and every input and output net must be on it. Nets from other
  domains come later, with synchronizers: two flip-flops for single bits, a
  handshake for buses (R11).
- **Size:** measured on the blinky milestone.

### Protocol (version 1)

The protocol works on 8-bit bytes. Read nets are packed, in map order, into
one little-endian bit vector, the read space; write nets likewise into the
write space. The map gives each net's bit offset and width. The host moves
bytes of those spaces.

Frames from the host: `0xA5`, command, payload length (one byte), payload,
CRC-8. Replies: `0x5A`, the same command, status, payload length, payload,
CRC-8. CRC-8 is polynomial 0x07, initial value 0, over every byte after the
sync byte. A frame with a bad CRC or an unknown command gets a status error,
not silence, so the host never waits for a timeout on a well-formed frame.

| Command | Code | Request payload | Reply payload |
| --- | --- | --- | --- |
| INFO | 0x01 | none | protocol version (1 byte), build ID (4 bytes, little-endian), read space bytes (2), write space bytes (2) |
| SNAPSHOT | 0x02 | none | none; all read nets latched in one cycle |
| READ | 0x03 | byte offset (2), count (1) | bytes of the snapshot |
| WRITE | 0x04 | byte offset (2), bytes | none; writes the write registers |
| OVERRIDE | 0x05 | byte offset (2), bytes of the enable mask | none; one enable bit per write net, in write-net order |
| CAPTURE\_\* (next phase) | 0x10 and up | arm, trigger, status, read | capture data in blocks |

Status: 0 OK, 1 bad CRC, 2 unknown command, 3 bad length or range.

At 115200 baud the link carries about 11.5 KB/s: reads feel instant, but an
8 KB capture would take about 0.7 s, which is why faster rates are measured
before the capture phase.

## Build integration

Decided: a yosys pre-pass in soft-agent that writes an instrumented copy of
the design, then the engine's `build` on a generated top. It needs no engine
change, and keeps the normal build untouched (R3).

```
 1. pre-pass (yosys): read user sources -> hierarchy, proc, flatten
                      -> expose read nets, expose -cut override nets
                      -> write_verilog  _build/debug/design_debug.v
 2. generate:         _build/debug/debug_top.v  (user top + debugger + muxes)
                      _build/debug/debug.pcf    (user pcf + UART pins)
                      _build/debug_map.json     (names, offsets, build ID)
 3. add:              the debugger's own Verilog, embedded in soft-agent
 4. engine build:     the engine's build tool on the files above
```

Steps 1 to 3 are new and live in soft-agent; step 4 is the engine's `build`
code, called with the generated files.

| Option | How | For | Against |
| --- | --- | --- | --- |
| I1. Wrapper only | Generate a top that instantiates the user's top and the debugger | Simplest; no netlist edits | Reaches only top-level ports, not internal nets |
| I2. Yosys pre-pass (chosen) | Run yosys to flatten and expose the nets, write Verilog, then build that with a generated top | Internal nets, reads and overrides; engine unchanged | Two yosys runs; errors point at generated files unless mapped back |
| I3. soft-agent's own build | Copy the engine's build steps with an extended synthesis script | One yosys run | Duplicates engine code that changes often; drifts like `main.rs` would |
| I4. Engine hook (later) | `build` accepts extra synthesis passes or a pre-synthesis step | Cleanest; one yosys run | An engine change: needs the user's go-ahead, and fits the move into the engine |

Tried on the blinky design (yosys 0.70, from the engine's package):
`flatten; expose w:count; expose -cut w:blink; write_verilog` gave
`output [22:0] count`, `output blink` and `input \blink.i`. So `-cut` gives
the design's value out and the loads' input in, and the generated top puts
the override mux between them.

- **When to expose:** right after `proc` and `flatten`, before optimization,
  so the nets still exist; a net missing then is error R9.
- **Errors in user terms:** the pre-pass runs on the user's own files first,
  so design errors keep their file and line.
- **Constraints:** the debug build writes its own copy of the user's
  constraint file with the debugger's TX and RX pins added. The user's file
  stays untouched.
- **Build ID:** a hash of the debug map and the source files, in the
  debugger's ROM and in `debug_map.json`.
- **Capture phase:** the same pre-pass exposes capture signals; the debugger
  adds a RAM block, so `debug_top.v` grows but the flow stays as drawn.

## Host side: MCP tools

A session model like the engine's serial tools, with names from the build's
debug map. All are soft-agent extension tools in the engine's style
(self-contained requests, absolute paths, errors as data):

| Tool | Does | Notes |
| --- | --- | --- |
| `debug_build` | Builds with a debug file: inserts the debugger, then the usual synthesis, place and route, packing | Same fields as `build`, plus `debug_file`; writes `debug_map.json` and reports the debugger's resources apart |
| `debug_open` | Opens a session on a board's debugger, from a build folder and a serial port | Reads the debugger's build ID first and refuses a mismatch (R8) |
| `debug_read` | Reads named nets, in one snapshot | Values as numbers and as bit strings, with widths |
| `debug_write` | Writes a named net, or releases an override back to the design | Refuses names that aren't outputs |
| `debug_close` | Ends the session | |

- **The port is shared.** The debugger owns the UART while a session is open,
  so the engine's `open_serial` on the same port is refused in version 1. A
  console channel in the protocol, so text the design prints still reaches a
  Serial Monitor, can come later.
- **Programming:** a debug session should pause while `program_board` writes
  the board and reconnect after, checking the build ID again. Version 1 may
  instead ask the host to close the session before programming.
- **Results:** in milestone 1 they show in Claude Code or any MCP client as
  structured tool results; request and reply, not a stream.
- **Capture phase:** `debug_capture` arms a capture with a trigger, and its
  result is a VCD file in the build folder, so the engine's `open_waveform`
  shows it with no new viewer.

## Prior art

What each existing tool suggests for this design. From memory, not checked
against current docs.

| Tool | How it works | Take from it |
| --- | --- | --- |
| AMD Xilinx ILA and VIO | Nets marked with a `mark_debug` attribute or picked in the tool; cores inserted into the synthesized netlist; JTAG uplink; a probes file ties the bitstream to names | Attribute marking (option B), a separate map file tied to the bitstream (R7, R8), VIO as the model for read/write |
| Intel SignalTap and In-System Sources and Probes | Nets picked in a settings file; capture with triggers into RAM; sources and probes for read/write; JTAG | Read/write and capture as two cores sharing one link; the file names the nets (option A) |
| Lattice Reveal | Inserter and analyzer tools; signals picked from the design; JTAG | The two-step model: insert, then analyze |
| LiteX LiteScope and UARTBone | A bus bridge over UART (UARTBone) gives the host register access; LiteScope captures into RAM behind it; `litex_server` shares the link with several clients | A UART bridge to a small register map, capture as a later block on the same bus, and one host process sharing the link |

Difference here: open source tools only (yosys, nextpnr), and the uplink is
the board's own USB UART, not JTAG, so it works on boards with no JTAG
adapter.

## Path into Logix Engine later

If the feature moves, the file format, debug map, protocol and tool names
move unchanged; only where the code lives changes. To keep that easy:

- **Follow the engine's tool conventions now:** self-contained requests,
  absolute paths, errors as `ToolError` data with categories, schemas
  generated from Rust types, display paths honored.
- **Keep the debugger's Verilog generic:** plain Verilog-2005 with no family
  primitives, so it builds on every toolchain the engine supports (R13).
- **Ship the debugger's Verilog like other engine data:** in the engine it
  would live in a package, not in the binary; soft-agent embeds it for now.
- **Name it neutrally:** `debug_build` could become a `debug_file` field of
  the engine's `build`, and the other tools keep their names. Avoid
  `soft_agent` in any name a host sees.
- **What the engine would need:** a hook for extra synthesis passes in
  `build::synth_script`, or a pre-synthesis step; today soft-agent works
  around its absence (Build integration). Proposing that hook is an engine
  change, so it needs the user's go-ahead first.

## Decisions

| # | Question | Decided |
| --- | --- | --- |
| Q1 | File format: JSON, JSONC, TOML or a `.pcf`-like text file? | JSONC with a generated schema |
| Q2 | Where nets are chosen: the file, the design, or both? | The debug file only for now; marking in the design later |
| Q3 | How the debugger gets in: a yosys pre-pass, soft-agent's own build, or an engine hook? | Yosys pre-pass now (no engine change); an engine hook when moving it |
| Q4 | Writes: `register` only first, or `override` of driven nets too? | Both, if yosys `expose -cut` works as expected; else `register` first |
| Q5 | Debugger clock: always the board clock, or any net? | One clock for the UART and the debugger, the board's clock resource by default; every input and output net must be on it |
| Q6 | The UART when the design uses it too? | Refuse in version 1; console channel later |
| Q7 | Baud rate? | 115200 now; measure the iCESugar's bridge at 1 Mbaud and up later |
| Q8 | Several debug files per project, merged? | One file per build request |
| Q9 | Where the schema lives? | In this repo, under `schemas/` |
| Q10 | Naming rules? | Full instance paths starting with the top module (`main.u_ctrl.state`), whole nets, generate names as yosys prints them, no wildcards (see Naming rules) |

## First milestone

On the iCESugar v1.5 with the blinky design, its counter moved into a
submodule (`u_counter`), on 2026-11-24:

- [x] Read three nets (`main.u_counter.count`, `main.u_counter.tick`,
  `main.blink`) named in
  a `debug.jsonc`. Two reads 0.26 s apart saw `count` advance 3.12 million
  cycles, as a 12 MHz clock does.
- [x] Write one net in `override` mode, then release it: the writes and the
  release succeed. Seeing the LED follow the host still needs a person
  watching the board.
- [x] The debug map and build ID check, with a stale-map refusal shown: a
  second debug build's folder was refused while the board ran the first.
- [x] Resource use of the debugger reported apart from the design's.
- [x] A normal `build` of the same sources still works, with no debugger in
  it.

Measured:

| Build | Logic cells (of 5280) | Max clock |
| --- | --- | --- |
| Blinky, normal build | 36 | 62.9 MHz |
| Blinky with the debugger (25 read bits, 1 override) | 706 | 31.5 MHz |
| The debugger alone (yosys, before placement) | 461 LUTs, 246 flip-flops | |

The debugger is above the 300-cell target and halves the blinky's maximum
clock. Both come mostly from its byte-wide muxes over the read and write
spaces (variable shifts) and the reply byte mux. Next steps for size and
speed: address the spaces a byte at a time through a shift register instead
of a variable shift, and register the reply byte.

## Implementation notes (version 1)

- **Where it lives:** `src/debug/` in soft-agent: `config.rs` (the debug
  file), `protocol.rs` (frames and the debug map), `build.rs`
  (`debug_build`), `session.rs` (the session tools), and `hdl/` (the
  debugger's Verilog and its testbench).
- **Families:** iCE40 only. The pre-pass loads the family's cell library
  (`+/ice40/cells_sim.v`) as black boxes, so `expose -cut` knows which ports of
  primitives such as `SB_RGBA_DRV` are inputs. Without it, a cut net kept
  feeding primitives from the design's side. Other families need their own
  library and constraint format.
- **Generated files:** `<build folder>/debug_src/` holds `design_debug.v`,
  `debug_top.v` (top module `logix_debug_top`, UART ports
  `logix_debug_tx`/`logix_debug_rx`), `debug.pcf` and `logix_debugger.v`.
  The engine's `build` builds them into the build folder, next to
  `debug_map.json`.
- **Sessions:** `debug_open` checks the build ID and the read and write
  sizes, then releases every override and clears the write registers, since
  the debugger can't be asked for them. `debug_close` releases them too.
  `debug_read` reads inputs only; to read a driven net, list it in `input`
  as well.
- **Limits:** a READ returns up to 128 bytes and a WRITE carries 6; the host
  splits larger spaces. A frame that stops arriving is dropped after about
  25 byte times. The port isn't locked against the engine's serial tools, so
  don't open both on one port.
- **Progress:** `debug_build` reports no progress while it runs yet.
