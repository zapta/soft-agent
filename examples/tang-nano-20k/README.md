# Tang Nano 20K: a running light with the debugger

The [Alhambra II example](../alhambra1) for the
[Sipeed Tang Nano 20K](https://wiki.sipeed.com/hardware/en/tang/Tang-Nano-20K/Nano-20K.html),
a Gowin GW2AR-18 board, made to test the debugger on Gowin (Apicula). The
catalog gives the board one user LED, so the running light steps through an
eight-place pattern inside the FPGA, and the LED shows place 0: it lights
for one step in eight. The debugger reaches the computer through the
board's BL616 USB serial port, over the USB-C cable that programs it.

With the board connected, from the repo's folder:

```bash
examples/tang-nano-20k/run.sh
```

It works as the Alhambra II example's `run.sh` does, with the same options;
its [TUTORIAL.md](../alhambra1/TUTORIAL.md) shows how to use the panel.

## The files

| File | What it is |
| --- | --- |
| `main.v` | The design: the running light, and a `ticker` module that makes the steps. |
| `pinout.cst` | Which FPGA pins the clock and the LED are on. |
| `debug.toml` | What the debugger watches and drives, and how it reaches the computer. |
| `logix.json` | The board and the top module, for the script. |
| `run.sh` | The one command above. |
