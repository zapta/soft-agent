# iCESugar v1.5: a running light with the debugger

The [Alhambra II example](../alhambra1) for the
[iCESugar v1.5](https://github.com/wuxx/icesugar), an iCE40 UltraPlus
(UP5K) board and the one the debugger is tested on first. The board has one
RGB LED, so the running light steps through an eight-place pattern inside
the FPGA, and the LED shows places 0 to 2: red, green, blue, then dark for
five steps. The debugger reaches the computer through the board's iCELink
USB serial port, over the USB-C cable that programs it.

With the board connected, from the repo's folder:

```bash
examples/icesugar-1-5/run.sh
```

It works as the Alhambra II example's `run.sh` does, with the same options;
its [TUTORIAL.md](../alhambra1/TUTORIAL.md) shows how to use the panel. Here
the panel also drives `colors`, the LED's three colors (blue, green, red
from the top bit down: `010` lights green), and watches `lit`, the colors as
the LED driver sees them.

If the LED shows blue where green is expected, the board is from an older
batch, which follows the schematic (blue on pin 39, green on 41): swap
`rgb_led[1]` and `rgb_led[2]` in `pinout.pcf`.

The debugger adds about 340 of the UP5K's 5280 logic cells (43 without it)
and one of its 30 RAM blocks; the design's maximum clock stays about 70 MHz.

## The files

| File | What it is |
| --- | --- |
| `main.v` | The design: the running light, the RGB LED driver, and a `ticker` module that makes the steps. |
| `pinout.pcf` | Which FPGA pins the clock and the RGB LED are on. |
| `debug.toml` | What the debugger watches and drives, and how it reaches the computer. |
| `logix.json` | The board and the top module, for the script. |
| `run.sh` | The one command above. |
