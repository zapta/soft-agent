// Logix debugger, protocol version 1 (docs/debugger.md, Debugger hardware).
//
// A register-map debugger behind a UART: the host sends frames, the
// debugger answers them. Plain Verilog-2005 with no FPGA primitives, so it
// builds for every family. One clock for everything (version 1).
//
// Host frame:  A5, command, length, payload (length bytes), CRC-8
// Reply frame: 5A, command, status, length, payload (length bytes), CRC-8
// CRC-8: polynomial 07, initial 00, over every byte after the sync byte.
//
// Commands: 01 INFO, 02 SNAPSHOT, 03 READ (offset lo, offset hi, count),
// 04 WRITE (offset lo, offset hi, bytes), 05 OVERRIDE (offset lo, offset hi,
// bytes of the enable mask), 06 READ_WRITES and 07 READ_ENABLES (offset lo,
// offset hi, count: the write registers and the override enables as the
// debugger holds them). Status: 0 OK, 1 bad CRC, 2 unknown command,
// 3 bad length or range.

module logix_debugger #(
    parameter integer CLKS_PER_BIT = 104,  // clock Hz / baud
    parameter integer READ_W = 1,          // bits of read nets, packed
    parameter integer WRITE_W = 1,         // bits of write nets, packed
    parameter integer OVR_W = 1,           // one enable per write net
    parameter [31:0] BUILD_ID = 32'h0
) (
    input  wire               clk,
    input  wire               rx,
    output wire               tx,
    input  wire [ READ_W-1:0] rd_in,
    output wire [WRITE_W-1:0] wr_out,
    output wire [  OVR_W-1:0] ovr_en
);

  localparam [7:0] PROTOCOL = 8'd2;
  localparam integer READ_BYTES = (READ_W + 7) / 8;
  localparam integer WRITE_BYTES = (WRITE_W + 7) / 8;
  localparam integer OVR_BYTES = (OVR_W + 7) / 8;
  localparam integer MAX_PAYLOAD = 8;  // request payload bytes kept
  localparam integer MAX_READ = 128;  // bytes per READ reply
  // A frame that stops arriving is dropped after about 25 byte times.
  localparam integer TIMEOUT = CLKS_PER_BIT * 256;
  localparam integer TW = $clog2(TIMEOUT + 1);

  // --- UART ---------------------------------------------------------------

  wire [7:0] uart_data;
  wire       uart_valid;
  reg  [7:0] tx_data = 8'h00;
  reg        tx_start = 1'b0;
  wire       tx_busy;

  logix_debugger_uart_rx #(.CLKS_PER_BIT(CLKS_PER_BIT)) u_rx (
      .clk  (clk),
      .rx   (rx),
      .data (uart_data),
      .valid(uart_valid)
  );

  logix_debugger_uart_tx #(.CLKS_PER_BIT(CLKS_PER_BIT)) u_tx (
      .clk  (clk),
      .data (tx_data),
      .start(tx_start),
      .tx   (tx),
      .busy (tx_busy)
  );

  // --- Registers ----------------------------------------------------------

  reg [ READ_BYTES*8-1:0] snap = 0;
  reg [WRITE_BYTES*8-1:0] wr_reg = 0;
  reg [  OVR_BYTES*8-1:0] en_reg = 0;

  assign wr_out = wr_reg[WRITE_W-1:0];
  assign ovr_en = en_reg[OVR_W-1:0];

  function [7:0] crc8;
    input [7:0] c;
    input [7:0] d;
    integer i;
    reg [7:0] x;
    begin
      x = c ^ d;
      for (i = 0; i < 8; i = i + 1) x = x[7] ? ({x[6:0], 1'b0} ^ 8'h07) : {x[6:0], 1'b0};
      crc8 = x;
    end
  endfunction

  // --- Command engine -----------------------------------------------------

  localparam [2:0] S_SYNC = 3'd0, S_CMD = 3'd1, S_LEN = 3'd2, S_PAY = 3'd3,
                   S_CRC = 3'd4, S_EXEC = 3'd5, S_WRITE = 3'd6, S_REPLY = 3'd7;

  reg [2:0] state = S_SYNC;

  // --- Receive buffer -----------------------------------------------------
  // The command engine takes input only while it waits for a frame, so bytes
  // that arrive while it executes or replies wait here. The host can then
  // send several frames at once and pay the link's round trip once.

  localparam integer FIFO_DEPTH = 64;
  reg [7:0] fifo[0:FIFO_DEPTH-1];
  reg [5:0] wp = 6'd0;
  reg [5:0] rp = 6'd0;
  reg [7:0] rx_data = 8'h00;  // the engine's next byte
  reg rx_valid = 1'b0;  // high for one clock per byte
  wire fifo_empty = (wp == rp);
  wire fifo_full = (wp + 1'b1 == rp);

  always @(posedge clk) begin
    if (uart_valid && !fifo_full) begin
      fifo[wp] <= uart_data;
      wp <= wp + 1'b1;
    end
    rx_valid <= 1'b0;
    if (state <= S_CRC && !fifo_empty && !rx_valid) begin
      rx_data  <= fifo[rp];
      rp       <= rp + 1'b1;
      rx_valid <= 1'b1;
    end
  end
  reg [7:0] cmd = 8'h00;
  reg [7:0] len = 8'h00;
  reg [7:0] pidx = 8'h00;
  reg [MAX_PAYLOAD*8-1:0] pay = 0;
  reg [7:0] crc = 8'h00;
  reg crc_ok = 1'b0;
  reg [TW-1:0] idle = 0;

  // The reply.
  reg [7:0] status = 8'h00;
  reg [7:0] rlen = 8'h00;
  reg [15:0] roff = 16'h0000;
  reg [8:0] ti = 9'd0;  // reply byte index
  reg [7:0] tcrc = 8'h00;
  reg sent = 1'b0;  // a byte was just handed to the transmitter

  // Writes, one byte a cycle.
  reg wsel = 1'b0;  // 0: write registers, 1: override enables
  reg [15:0] woff = 16'h0000;
  reg [7:0] wn = 8'h00;
  reg [7:0] wi = 8'h00;

  wire [15:0] p_off = {pay[15:8], pay[7:0]};
  wire [7:0] p_count = pay[23:16];
  wire [7:0] p_byte = pay >> ((wi + 8'd2) * 8);
  wire [7:0] snap_byte = snap >> ((roff + ti - 9'd4) * 8);
  wire [7:0] wr_byte = wr_reg >> ((roff + ti - 9'd4) * 8);
  wire [7:0] en_byte = en_reg >> ((roff + ti - 9'd4) * 8);

  // The reply's byte number ti.
  reg [7:0] reply_byte;
  always @* begin
    if (ti == 0) reply_byte = 8'h5A;
    else if (ti == 1) reply_byte = cmd;
    else if (ti == 2) reply_byte = status;
    else if (ti == 3) reply_byte = rlen;
    else if (ti == 9'd4 + rlen) reply_byte = tcrc;
    else if (cmd == 8'h01)
      case (ti)
        9'd4: reply_byte = PROTOCOL;
        9'd5: reply_byte = BUILD_ID[7:0];
        9'd6: reply_byte = BUILD_ID[15:8];
        9'd7: reply_byte = BUILD_ID[23:16];
        9'd8: reply_byte = BUILD_ID[31:24];
        9'd9: reply_byte = READ_BYTES[7:0];
        9'd10: reply_byte = READ_BYTES[15:8];
        9'd11: reply_byte = WRITE_BYTES[7:0];
        default: reply_byte = WRITE_BYTES[15:8];
      endcase
    else if (cmd == 8'h06) reply_byte = wr_byte;
    else if (cmd == 8'h07) reply_byte = en_byte;
    else reply_byte = snap_byte;
  end

  integer j;
  always @(posedge clk) begin
    tx_start <= 1'b0;

    // Frames that stop arriving mid-way are dropped.
    if (state == S_SYNC || rx_valid) idle <= 0;
    else if (state <= S_CRC) idle <= idle + 1'b1;

    if (state <= S_CRC && idle == TIMEOUT[TW-1:0]) begin
      state <= S_SYNC;
    end else
      case (state)
        S_SYNC:
        if (rx_valid && rx_data == 8'hA5) begin
          crc   <= 8'h00;
          state <= S_CMD;
        end
        S_CMD:
        if (rx_valid) begin
          cmd   <= rx_data;
          crc   <= crc8(crc, rx_data);
          state <= S_LEN;
        end
        S_LEN:
        if (rx_valid) begin
          len   <= rx_data;
          crc   <= crc8(crc, rx_data);
          pidx  <= 8'h00;
          pay   <= 0;
          state <= (rx_data == 8'h00) ? S_CRC : S_PAY;
        end
        S_PAY:
        if (rx_valid) begin
          for (j = 0; j < MAX_PAYLOAD; j = j + 1) if (pidx == j) pay[j*8+:8] <= rx_data;
          crc  <= crc8(crc, rx_data);
          pidx <= pidx + 1'b1;
          if (pidx + 1'b1 == len) state <= S_CRC;
        end
        S_CRC:
        if (rx_valid) begin
          crc_ok <= (rx_data == crc);
          state  <= S_EXEC;
        end
        S_EXEC: begin
          status <= 8'd0;
          rlen <= 8'd0;
          ti <= 9'd0;
          tcrc <= 8'h00;
          sent <= 1'b0;
          state <= S_REPLY;
          if (!crc_ok) status <= 8'd1;
          else if (len > MAX_PAYLOAD) status <= 8'd3;
          else
            case (cmd)
              8'h01:
              if (len != 0) status <= 8'd3;
              else rlen <= 8'd9;
              8'h02:
              if (len != 0) status <= 8'd3;
              else snap <= rd_in;
              8'h03, 8'h06, 8'h07:
              if (len != 3 || p_count > MAX_READ
                  || p_off + p_count > (cmd == 8'h03 ? READ_BYTES : cmd == 8'h06 ? WRITE_BYTES : OVR_BYTES))
                status <= 8'd3;
              else begin
                rlen <= p_count;
                roff <= p_off;
              end
              8'h04, 8'h05:
              if (len < 3 || p_off + (len - 8'd2) > (cmd == 8'h04 ? WRITE_BYTES : OVR_BYTES))
                status <= 8'd3;
              else begin
                wsel  <= (cmd == 8'h05);
                woff  <= p_off;
                wn    <= len - 8'd2;
                wi    <= 8'd0;
                state <= S_WRITE;
              end
              default: status <= 8'd2;
            endcase
        end
        S_WRITE: begin
          for (j = 0; j < WRITE_BYTES; j = j + 1)
          if (!wsel && woff + wi == j) wr_reg[j*8+:8] <= p_byte;
          for (j = 0; j < OVR_BYTES; j = j + 1) if (wsel && woff + wi == j) en_reg[j*8+:8] <= p_byte;
          wi <= wi + 1'b1;
          if (wi + 1'b1 == wn) state <= S_REPLY;
        end
        S_REPLY:
        if (sent) sent <= 1'b0;  // let busy rise first
        else if (!tx_busy) begin
          tx_data  <= reply_byte;
          tx_start <= 1'b1;
          sent     <= 1'b1;
          if (ti != 0) tcrc <= crc8(tcrc, reply_byte);
          if (ti == 9'd4 + rlen) state <= S_SYNC;
          else ti <= ti + 1'b1;
        end
        default: state <= S_SYNC;
      endcase
  end

endmodule

// 8N1 receiver: samples each bit at its middle.
module logix_debugger_uart_rx #(
    parameter integer CLKS_PER_BIT = 104
) (
    input  wire       clk,
    input  wire       rx,
    output reg  [7:0] data  = 8'h00,
    output reg        valid = 1'b0
);
  localparam integer CW = $clog2(CLKS_PER_BIT * 2);
  reg r1 = 1'b1, r2 = 1'b1;
  reg busy = 1'b0;
  reg [CW-1:0] cnt = 0;
  reg [3:0] n = 4'd0;
  reg [7:0] sh = 8'h00;

  always @(posedge clk) begin
    r1 <= rx;
    r2 <= r1;
    valid <= 1'b0;
    if (!busy) begin
      if (!r2) begin
        busy <= 1'b1;
        cnt  <= CLKS_PER_BIT / 2;  // to the middle of the start bit
        n    <= 4'd0;
      end
    end else if (cnt != 0) cnt <= cnt - 1'b1;
    else begin
      cnt <= CLKS_PER_BIT - 1;
      n   <= n + 1'b1;
      if (n == 0) begin
        if (r2) busy <= 1'b0;  // a glitch, not a start bit
      end else if (n <= 8) sh <= {r2, sh[7:1]};
      else begin
        busy <= 1'b0;
        if (r2) begin  // a good stop bit
          data  <= sh;
          valid <= 1'b1;
        end
      end
    end
  end
endmodule

// 8N1 transmitter. busy stays high until the stop bit is done.
module logix_debugger_uart_tx #(
    parameter integer CLKS_PER_BIT = 104
) (
    input  wire       clk,
    input  wire [7:0] data,
    input  wire       start,
    output reg        tx   = 1'b1,
    output wire       busy
);
  localparam integer CW = $clog2(CLKS_PER_BIT + 1);
  reg [CW-1:0] cnt = 0;
  reg [3:0] n = 4'd0;
  reg [9:0] sh = 10'h3FF;

  assign busy = (n != 0) || (cnt != 0);

  always @(posedge clk) begin
    if (cnt != 0) cnt <= cnt - 1'b1;
    else if (n != 0) begin
      tx  <= sh[0];
      sh  <= {1'b1, sh[9:1]};
      n   <= n - 1'b1;
      cnt <= CLKS_PER_BIT - 1;
    end else if (start) begin
      sh <= {1'b1, data, 1'b0};
      n  <= 4'd10;
    end
  end
endmodule
