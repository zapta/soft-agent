// Testbench for logix_debugger.v: plays the host over the UART, a few
// clocks per bit, and checks every command's reply and effect.
//
// Run with Icarus Verilog (iverilog and vvp are in the engine's toolchain):
//   iverilog -g2005 -o tb.vvp logix_debugger_tb.v logix_debugger.v && vvp tb.vvp

`timescale 1ns / 1ps

module logix_debugger_tb;

  localparam integer CPB = 8;
  localparam [31:0] ID = 32'hC0DE1234;

  reg clk = 0;
  always #5 clk = ~clk;

  reg host_tx = 1;  // to the debugger's rx
  wire host_rx;
  reg [19:0] rd_in = 20'hABCDE;
  wire [11:0] wr_out;
  wire [1:0] ovr_en;

  logix_debugger #(
      .CLKS_PER_BIT(CPB),
      .READ_W(20),
      .WRITE_W(12),
      .OVR_W(2),
      .BUILD_ID(ID)
  ) dut (
      .clk(clk),
      .rx(host_tx),
      .tx(host_rx),
      .rd_in(rd_in),
      .wr_out(wr_out),
      .ovr_en(ovr_en)
  );

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

  task send_byte(input [7:0] b);
    integer i;
    begin
      host_tx = 0;
      repeat (CPB) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin
        host_tx = b[i];
        repeat (CPB) @(posedge clk);
      end
      host_tx = 1;
      repeat (CPB) @(posedge clk);
    end
  endtask

  task recv_byte(output [7:0] b);
    integer i;
    begin
      @(negedge host_rx);
      repeat (CPB / 2) @(posedge clk);
      for (i = 0; i < 8; i = i + 1) begin
        repeat (CPB) @(posedge clk);
        b[i] = host_rx;
      end
      repeat (CPB) @(posedge clk);
      if (host_rx !== 1) $fatal(1, "bad stop bit");
    end
  endtask

  reg [7:0] req[0:15];
  reg [7:0] rep[0:255];
  integer rep_len, rep_status;

  // Sends a frame of n payload bytes from req, with a CRC (bad if
  // corrupt), and reads the reply into rep.
  // Sends a frame of n payload bytes from req, with a CRC (bad if corrupt).
  task send_frame(input [7:0] command, input integer n, input corrupt);
    integer i;
    reg [7:0] c;
    begin
      send_byte(8'hA5);
      c = crc8(8'h00, command);
      send_byte(command);
      c = crc8(c, n[7:0]);
      send_byte(n[7:0]);
      for (i = 0; i < n; i = i + 1) begin
        c = crc8(c, req[i]);
        send_byte(req[i]);
      end
      send_byte(corrupt ? ~c : c);
    end
  endtask

  // Reads a reply into rep, checking its sync, command and CRC.
  task get_reply(input [7:0] command);
    integer i;
    reg [7:0] c, b;
    begin
      recv_byte(b);
      if (b !== 8'h5A) $fatal(1, "reply sync %h", b);
      recv_byte(b);
      if (b !== command) $fatal(1, "reply command %h, expected %h", b, command);
      c = crc8(8'h00, b);
      recv_byte(b);
      rep_status = b;
      c = crc8(c, b);
      recv_byte(b);
      rep_len = b;
      c = crc8(c, b);
      for (i = 0; i < rep_len; i = i + 1) begin
        recv_byte(rep[i]);
        c = crc8(c, rep[i]);
      end
      recv_byte(b);
      if (b !== c) $fatal(1, "reply CRC %h, expected %h", b, c);
    end
  endtask

  // A frame, then its reply.
  task frame(input [7:0] command, input integer n, input corrupt);
    begin
      send_frame(command, n, corrupt);
      get_reply(command);
    end
  endtask

  task expect_status(input integer s, input integer n);
    begin
      if (rep_status !== s) $fatal(1, "status %0d, expected %0d", rep_status, s);
      if (rep_len !== n) $fatal(1, "reply length %0d, expected %0d", rep_len, n);
    end
  endtask

  initial begin
    repeat (20) @(posedge clk);

    // INFO
    frame(8'h01, 0, 0);
    expect_status(0, 9);
    if (rep[0] !== 2) $fatal(1, "protocol %0d", rep[0]);
    if ({rep[4], rep[3], rep[2], rep[1]} !== ID) $fatal(1, "build ID");
    if ({rep[6], rep[5]} !== 3 || {rep[8], rep[7]} !== 2) $fatal(1, "sizes");

    // SNAPSHOT, then the nets change, then READ sees the snapshot.
    frame(8'h02, 0, 0);
    expect_status(0, 0);
    rd_in = 20'h12345;
    req[0] = 0; req[1] = 0; req[2] = 3;
    frame(8'h03, 3, 0);
    expect_status(0, 3);
    if ({rep[2], rep[1], rep[0]} !== 24'h0ABCDE) $fatal(1, "read %h%h%h", rep[2], rep[1], rep[0]);
    req[0] = 1; req[1] = 0; req[2] = 2;
    frame(8'h03, 3, 0);
    expect_status(0, 2);
    if ({rep[1], rep[0]} !== 16'h0ABC) $fatal(1, "read at offset");

    // READ past the end.
    req[0] = 2; req[1] = 0; req[2] = 2;
    frame(8'h03, 3, 0);
    expect_status(3, 0);

    // WRITE two bytes, then one at an offset.
    req[0] = 0; req[1] = 0; req[2] = 8'h21; req[3] = 8'h43;
    frame(8'h04, 4, 0);
    expect_status(0, 0);
    if (wr_out !== 12'h321) $fatal(1, "wr_out %h", wr_out);
    req[0] = 1; req[1] = 0; req[2] = 8'h0F;
    frame(8'h04, 3, 0);
    if (wr_out !== 12'hF21) $fatal(1, "wr_out %h", wr_out);

    // READ_WRITES gives back what was written.
    req[0] = 0; req[1] = 0; req[2] = 2;
    frame(8'h06, 3, 0);
    expect_status(0, 2);
    if ({rep[1], rep[0]} !== 16'h0F21) $fatal(1, "read back writes %h%h", rep[1], rep[0]);
    req[0] = 1; req[1] = 0; req[2] = 2;
    frame(8'h06, 3, 0);
    expect_status(3, 0);

    // OVERRIDE on and off, each read back with READ_ENABLES.
    req[0] = 0; req[1] = 0; req[2] = 8'h02;
    frame(8'h05, 3, 0);
    expect_status(0, 0);
    if (ovr_en !== 2'b10) $fatal(1, "ovr_en %b", ovr_en);
    req[0] = 0; req[1] = 0; req[2] = 1;
    frame(8'h07, 3, 0);
    expect_status(0, 1);
    if (rep[0] !== 8'h02) $fatal(1, "read back enables %h", rep[0]);
    req[2] = 8'h00;
    frame(8'h05, 3, 0);
    if (ovr_en !== 2'b00) $fatal(1, "ovr_en %b", ovr_en);

    // Bad CRC, unknown command, bad length.
    frame(8'h01, 0, 1);
    expect_status(1, 0);
    frame(8'h7E, 0, 0);
    expect_status(2, 0);
    frame(8'h01, 1, 0);
    expect_status(3, 0);

    // A frame cut short is dropped, and the next one works.
    send_byte(8'hA5);
    send_byte(8'h01);
    repeat (CPB * 300) @(posedge clk);
    frame(8'h01, 0, 0);
    expect_status(0, 9);

    // INFO, SNAPSHOT and READ sent back to back, without waiting: the
    // receive buffer holds what arrives while a reply goes out.
    rd_in = 20'h5A5A5;
    req[0] = 0; req[1] = 0; req[2] = 3;
    fork
      begin
        send_frame(8'h01, 0, 0);
        send_frame(8'h02, 0, 0);
        send_frame(8'h03, 3, 0);
      end
      begin
        get_reply(8'h01);
        expect_status(0, 9);
        get_reply(8'h02);
        expect_status(0, 0);
        get_reply(8'h03);
        expect_status(0, 3);
        if ({rep[2], rep[1], rep[0]} !== 24'h05A5A5) $fatal(1, "batched read %h%h%h", rep[2], rep[1], rep[0]);
      end
    join

    $display("OK: logix_debugger passes");
    $finish;
  end

  initial begin
    #50_000_000;
    $fatal(1, "timeout");
  end

endmodule
