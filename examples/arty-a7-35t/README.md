# Arty A7-35T: a running light with the debugger

The [Alhambra II example](../alhambra1) for the
[Digilent Arty A7-35T](https://digilent.com/reference/programmable-logic/arty-a7/start),
a Xilinx Artix-7 board, made to test the debugger on Xilinx 7-series
(openXC7). The light runs across the board's four green LEDs. The debugger
reaches the computer through the board's FTDI USB serial port, over the
micro-USB cable that programs it.

With the board connected, from the repo's folder:

```bash
examples/arty-a7-35t/run.sh
```

It works as the Alhambra II example's `run.sh` does, with the same options;
its [TUTORIAL.md](../alhambra1/TUTORIAL.md) shows how to use the panel.

## The files

| File | What it is |
| --- | --- |
| `main.v` | The design: the running light, and a `ticker` module that makes the steps. |
| `pinout.xdc` | Which FPGA pins the clock and the LEDs are on. |
| `debug.toml` | What the debugger watches and drives, and how it reaches the computer. |
| `logix.json` | The board and the top module, for the script. |
| `run.sh` | The one command above. |
