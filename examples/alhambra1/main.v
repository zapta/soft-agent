// A running light for the Alhambra II: one of the eight LEDs lights at a
// time and steps along the row four times a second, from the board's
// 12 MHz clock.
//
// It is ready for the debugger: debug.toml names the nets to watch and drive,
// and `enable_step` below is a spare control input for the debugger to
// drive.

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

  // Each step waits for it. The design keeps it 1, so the light runs; in a
  // debug build the debugger can take it over from the panel (debug.toml).
  wire enable_step = 1'b1;

  // The lit LED, moving one place left at each step and wrapping around.
  reg [7:0] pattern = 8'b0000_0001;

  // A step is due from the timer's tick until it is taken. Ticks that come
  // while one is due don't add up.
  reg  due = 1'b0;
  wire take = due && enable_step;

  always @(posedge sys_clk) begin
    if (take) pattern <= {pattern[6:0], pattern[7]};
    due <= step || (due && !take);
  end

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
