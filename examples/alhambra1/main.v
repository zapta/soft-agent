// A running light for the Alhambra II: one of the eight LEDs lights at a
// time and steps along the row four times a second, from the board's
// 12 MHz clock.
//
// It is ready for the debugger: debug.toml names the nets to watch and drive,
// and `freeze` below is a source net, driven only by the debugger in a debug
// build.

module main #(
    // Clock cycles per step: 3,000,000 at 12 MHz is a quarter second.
    parameter integer STEP = 3_000_000
) (
    input  wire       sys_clk,  // 12 MHz
    output wire [7:0] leds      // LED7 to LED0, active high
);

  wire step;

  ticker #(.PERIOD(STEP)) u_ticker (
      .clk (sys_clk),
      .tick(step)
  );

  // Stops the light while 1. In a debug build (debug_build defines
  // LOGIX_DEBUG) nothing in the design drives it: the debugger does, from
  // the panel (debug.toml, mode = "source"). In a normal build it is 0.
`ifdef LOGIX_DEBUG
  wire freeze;
`else
  wire freeze = 1'b0;
`endif

  // The lit LED, moving one place left at each step and wrapping around.
  reg [7:0] pattern = 8'b0000_0001;

  always @(posedge sys_clk) if (step && !freeze) pattern <= {pattern[6:0], pattern[7]};

  assign leds = pattern;

endmodule

// Pulses tick for one clock every PERIOD clocks.
module ticker #(
    parameter integer PERIOD = 3_000_000
) (
    input  wire clk,
    output reg  tick = 0
);

  localparam integer WIDTH = $clog2(PERIOD);
  localparam [WIDTH-1:0] LAST = PERIOD[WIDTH-1:0] - 1'b1;

  reg [WIDTH-1:0] count = 0;

  always @(posedge clk) begin
    tick <= 1'b0;
    if (count == LAST) begin
      count <= 0;
      tick  <= 1'b1;
    end else begin
      count <= count + 1'b1;
    end
  end

endmodule
