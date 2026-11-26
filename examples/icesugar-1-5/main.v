// A running light for the iCESugar v1.5. The board has one RGB LED, so the
// light runs through an eight-place pattern inside the FPGA, stepping four
// times a second from the board's 12 MHz clock, and the LED shows places 0
// to 2 as red, green and blue: red, green, blue, then dark for five steps.
//
// It is ready for the debugger: debug.toml names the nets to watch and drive,
// and `enable_step` below is a spare control input for the debugger to
// drive.

module main #(
    // Clock cycles per step: 3,000,000 at 12 MHz is a quarter second.
    parameter integer STEP = 3_000_000
) (
    input  wire       sys_clk,  // 12 MHz
    output wire [2:0] rgb_led   // red, green, blue; lit when 0
);

  wire step;

  ticker #(.PERIOD(STEP)) u_ticker (
      .clk (sys_clk),
      .tick(step)
  );

  // Each step waits for it. The design keeps it 1, so the light runs; in a
  // debug build the debugger can take it over from the panel (debug.toml).
  wire enable_step = 1'b1;

  // The lit place, moving one place left at each step and wrapping around.
  reg [7:0] pattern = 8'b0000_0001;

  // A step is due from the timer's tick until it is taken. Ticks that come
  // while one is due don't add up.
  reg  due = 1'b0;
  wire take = due && enable_step;

  always @(posedge sys_clk) begin
    if (take) pattern <= {pattern[6:0], pattern[7]};
    due <= step || (due && !take);
  end

  // The colors to light: red, green, blue from places 0, 1 and 2. In a debug
  // build the debugger can take them over (debug.toml).
  wire [2:0] rgb_on = pattern[2:0];

  // The RGB LED is on the FPGA's RGB LED driver pads, which only
  // SB_RGBA_DRV, the iCE40 UltraPlus's LED driver cell, can drive. It pulls
  // a pad low (lighting its color) while its PWM input is 1.
`ifdef SYNTHESIZE
  // The driver's pads: green on RGB0, red on RGB1, blue on RGB2.
  SB_RGBA_DRV #(
      .CURRENT_MODE("0b1"),       // half current
      .RGB0_CURRENT("0b000001"),  // the lowest current, bright enough
      .RGB1_CURRENT("0b000001"),
      .RGB2_CURRENT("0b000001")
  ) rgb_driver (
      .CURREN  (1'b1),
      .RGBLEDEN(1'b1),
      .RGB0PWM (rgb_on[1]),
      .RGB1PWM (rgb_on[0]),
      .RGB2PWM (rgb_on[2]),
      .RGB0    (rgb_led[1]),
      .RGB1    (rgb_led[0]),
      .RGB2    (rgb_led[2])
  );
`else
  // Simulation only: the levels the driver gives the pads.
  assign rgb_led = ~rgb_on;
`endif

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
